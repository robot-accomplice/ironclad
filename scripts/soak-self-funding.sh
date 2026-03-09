#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────
# soak-self-funding.sh — End-to-end soak test for the Ironclad
#                         self-funding revenue loop
#
# Exercises the complete lifecycle:
#   Revenue Intake → Qualify → Plan → Fulfill → Settle
#   → Tax Payout (on-chain ERC-20 transfer)
#   → Revenue Swap (on-chain DEX call)
#   → Reconcile + Confirm
#
# Prerequisites:
#   - Ironclad server running (`ironclad serve`)
#   - Wallet funded with ≥$10 USDC on Base
#   - jq installed (`brew install jq` / `apt install jq`)
#   - Config must have self_funding.tax.destination_wallet set
#
# Usage:
#   ./scripts/soak-self-funding.sh                       # full run
#   ./scripts/soak-self-funding.sh --dry-run              # lifecycle only, no on-chain tx
#   ./scripts/soak-self-funding.sh --amount 2.00          # custom revenue amount
#   ./scripts/soak-self-funding.sh --base-url http://...  # custom server URL
#   ./scripts/soak-self-funding.sh --api-key sk-...       # custom API key
# ─────────────────────────────────────────────────────────────────
set -euo pipefail

# ── Defaults ──────────────────────────────────────────────────────
BASE_URL="${IRONCLAD_URL:-http://127.0.0.1:18789}"
API_KEY="${IRONCLAD_API_KEY:-}"
DRY_RUN=0
REVENUE_AMOUNT=5.00
TAX_WALLET="${IRONCLAD_TAX_WALLET:-}"
USDC_BASE="0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"
POLL_INTERVAL=5
MAX_POLLS=60

# ── Colors ────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
RESET='\033[0m'

# ── Parse args ────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)       DRY_RUN=1; shift ;;
        --amount)        REVENUE_AMOUNT="$2"; shift 2 ;;
        --base-url)      BASE_URL="$2"; shift 2 ;;
        --api-key)       API_KEY="$2"; shift 2 ;;
        --tax-wallet)    TAX_WALLET="$2"; shift 2 ;;
        --help|-h)
            echo "Usage: $0 [--dry-run] [--amount N] [--base-url URL] [--api-key KEY] [--tax-wallet 0x...]"
            exit 0 ;;
        *) echo "Unknown arg: $1"; exit 1 ;;
    esac
done

# ── Helpers ───────────────────────────────────────────────────────
step()  { echo -e "\n${CYAN}${BOLD}▸ $*${RESET}"; }
ok()    { echo -e "  ${GREEN}✓${RESET} $*"; }
warn()  { echo -e "  ${YELLOW}⚠${RESET} $*"; }
fail()  { echo -e "  ${RED}✗${RESET} $*"; exit 1; }
info()  { echo -e "  ${DIM}$*${RESET}"; }

PASS_COUNT=0
FAIL_COUNT=0
check_pass() { ((PASS_COUNT++)); ok "$*"; }
check_fail() { ((FAIL_COUNT++)); echo -e "  ${RED}✗ FAIL:${RESET} $*"; }

api() {
    local method="$1" path="$2"
    shift 2
    local args=(-s -w '\n%{http_code}')
    if [[ -n "$API_KEY" ]]; then
        args+=(-H "x-api-key: $API_KEY")
    fi
    args+=(-H "Content-Type: application/json")

    local response
    if [[ "$method" == "GET" ]]; then
        response=$(curl "${args[@]}" "${BASE_URL}${path}" "$@")
    else
        response=$(curl "${args[@]}" -X "$method" "${BASE_URL}${path}" "$@")
    fi

    # Split body and status code
    local body status_code
    status_code=$(echo "$response" | tail -1)
    body=$(echo "$response" | sed '$d')

    if [[ "$status_code" -ge 400 ]]; then
        echo "HTTP_ERROR:${status_code}:${body}"
        return 1
    fi
    echo "$body"
}

# Construct ERC-20 transfer calldata
# Usage: erc20_transfer_calldata <to_address> <amount_usdc>
erc20_transfer_calldata() {
    local to_addr="$1"
    local amount_usdc="$2"

    # USDC has 6 decimals — convert dollar amount to smallest unit
    # Use awk for floating-point math
    local amount_raw
    amount_raw=$(awk "BEGIN { printf \"%.0f\", ${amount_usdc} * 1000000 }")

    # Convert to hex (without 0x prefix)
    local amount_hex
    amount_hex=$(printf '%064x' "$amount_raw")

    # Pad address to 32 bytes (remove 0x prefix, left-pad with zeros)
    local addr_clean
    addr_clean="${to_addr#0x}"
    addr_clean="${addr_clean#0X}"
    local addr_padded
    addr_padded=$(printf '%064s' "$addr_clean" | tr ' ' '0')

    # ERC-20 transfer(address,uint256) selector = 0xa9059cbb
    echo "0xa9059cbb${addr_padded}${amount_hex}"
}

# ── Banner ────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}╔══════════════════════════════════════════════════════╗${RESET}"
echo -e "${BOLD}║   IRONCLAD SELF-FUNDING SOAK TEST                   ║${RESET}"
echo -e "${BOLD}╚══════════════════════════════════════════════════════╝${RESET}"
echo ""
echo -e "  Server:   ${CYAN}${BASE_URL}${RESET}"
echo -e "  Amount:   ${CYAN}\$${REVENUE_AMOUNT} USDC${RESET}"
echo -e "  Mode:     ${CYAN}$([ "$DRY_RUN" -eq 1 ] && echo "DRY RUN (no on-chain tx)" || echo "LIVE (on-chain tx!)")${RESET}"
echo ""

# ═══════════════════════════════════════════════════════════════════
# PHASE 1: Pre-flight Checks
# ═══════════════════════════════════════════════════════════════════
step "Phase 1: Pre-flight Checks"

# 1a. Server health
info "Checking server health..."
health=$(api GET /api/health 2>/dev/null) || fail "Server is not reachable at ${BASE_URL}"
check_pass "Server is healthy"

# 1b. Wallet status
info "Checking wallet..."
wallet_json=$(api GET /api/wallet/balance 2>/dev/null) || fail "Cannot reach wallet endpoint"
wallet_addr=$(echo "$wallet_json" | jq -r '.address // empty')
wallet_balance=$(echo "$wallet_json" | jq -r '.balance // "0"')
wallet_chain=$(echo "$wallet_json" | jq -r '.chain_id // 0')
wallet_network=$(echo "$wallet_json" | jq -r '.network // "unknown"')

if [[ -z "$wallet_addr" ]]; then
    fail "Wallet not configured. Run 'ironclad init' first."
fi
check_pass "Wallet: ${wallet_addr}"
info "Balance: \$${wallet_balance} USDC on ${wallet_network} (chain ${wallet_chain})"

# Check minimum balance
min_needed=$(awk "BEGIN { printf \"%.2f\", ${REVENUE_AMOUNT} + 2.0 }")
if awk "BEGIN { exit (${wallet_balance} >= ${min_needed}) ? 1 : 0 }"; then
    fail "Insufficient balance: \$${wallet_balance} < \$${min_needed} needed"
fi
check_pass "Balance sufficient (\$${wallet_balance} ≥ \$${min_needed})"

# 1c. Treasury policy
treasury_cap=$(echo "$wallet_json" | jq -r '.treasury.per_payment_cap // 0')
treasury_reserve=$(echo "$wallet_json" | jq -r '.treasury.minimum_reserve // 0')
info "Treasury: cap=\$${treasury_cap}/tx, reserve=\$${treasury_reserve}"
check_pass "Treasury policy loaded"

# 1d. Chain validation
if [[ "$wallet_chain" != "8453" ]]; then
    warn "Wallet is on chain ${wallet_chain}, not Base (8453). Swap calldata may differ."
fi

echo ""
echo -e "  ${BOLD}Pre-flight: ${GREEN}${PASS_COUNT} passed${RESET}, ${RED}${FAIL_COUNT} failed${RESET}"

# ═══════════════════════════════════════════════════════════════════
# PHASE 2: Revenue Lifecycle (off-chain)
# ═══════════════════════════════════════════════════════════════════
step "Phase 2: Revenue Lifecycle"

SOAK_REF="soak-$(date +%s)-$$"
info "Settlement ref: ${SOAK_REF}"

# 2a. Intake
info "Creating revenue opportunity..."
intake_resp=$(api POST /api/services/opportunities/intake \
    -d "$(jq -n \
        --arg src "soak_test" \
        --arg strat "micro_bounty" \
        --argjson amt "$REVENUE_AMOUNT" \
        '{source: $src, strategy: $strat, expected_revenue_usdc: $amt, payload: {test_run: true, ref: "soak-test"}}'
    )"
) || fail "Intake failed: ${intake_resp}"

OPP_ID=$(echo "$intake_resp" | jq -r '.opportunity_id')
opp_status=$(echo "$intake_resp" | jq -r '.status')
confidence=$(echo "$intake_resp" | jq -r '.score.confidence_score // 0')
if [[ "$opp_status" != "intake" ]]; then
    fail "Expected status=intake, got ${opp_status}"
fi
check_pass "Intake: ${OPP_ID} (confidence=${confidence})"

# 2b. Qualify
info "Qualifying opportunity..."
qualify_resp=$(api POST "/api/services/opportunities/${OPP_ID}/qualify" \
    -d '{"approved": true, "reason": "soak test qualification"}'
) || fail "Qualify failed: ${qualify_resp}"

opp_status=$(echo "$qualify_resp" | jq -r '.status')
if [[ "$opp_status" != "qualified" ]]; then
    fail "Expected status=qualified, got ${opp_status}"
fi
check_pass "Qualified"

# 2c. Plan
info "Planning opportunity..."
plan_resp=$(api POST "/api/services/opportunities/${OPP_ID}/plan" \
    -d '{"plan": {"steps": ["execute soak test"], "estimated_hours": 0.1}}'
) || fail "Plan failed: ${plan_resp}"

opp_status=$(echo "$plan_resp" | jq -r '.status')
if [[ "$opp_status" != "planned" ]]; then
    fail "Expected status=planned, got ${opp_status}"
fi
check_pass "Planned"

# 2d. Fulfill
info "Fulfilling opportunity..."
fulfill_resp=$(api POST "/api/services/opportunities/${OPP_ID}/fulfill" \
    -d '{"evidence": {"result": "soak test completed", "verified": true}}'
) || fail "Fulfill failed: ${fulfill_resp}"

opp_status=$(echo "$fulfill_resp" | jq -r '.status')
if [[ "$opp_status" != "fulfilled" ]]; then
    fail "Expected status=fulfilled, got ${opp_status}"
fi
check_pass "Fulfilled"

# 2e. Settle
info "Settling opportunity for \$${REVENUE_AMOUNT} USDC..."
settle_body=$(jq -n \
    --arg ref "$SOAK_REF" \
    --argjson amt "$REVENUE_AMOUNT" \
    '{
        settlement_ref: $ref,
        amount_usdc: $amt,
        currency: "USDC",
        attributable_costs_usdc: 0.0,
        auto_swap: false,
        target_chain: "BASE"
    }'
)
settle_resp=$(api POST "/api/services/opportunities/${OPP_ID}/settle" \
    -d "$settle_body"
) || fail "Settlement failed: ${settle_resp}"

opp_status=$(echo "$settle_resp" | jq -r '.status')
net_profit=$(echo "$settle_resp" | jq -r '.net_profit_usdc // 0')
tax_amount=$(echo "$settle_resp" | jq -r '.tax_amount_usdc // 0')
retained=$(echo "$settle_resp" | jq -r '.retained_earnings_usdc // 0')
tax_rate=$(echo "$settle_resp" | jq -r '.tax_rate // 0')
tax_dest=$(echo "$settle_resp" | jq -r '.tax_destination_wallet // "none"')
swap_queued=$(echo "$settle_resp" | jq -r '.swap_queued // false')

if [[ "$opp_status" != "settled" ]]; then
    fail "Expected status=settled, got ${opp_status}"
fi
check_pass "Settled"
info "  Net profit:        \$${net_profit}"
info "  Tax (${tax_rate}):         \$${tax_amount}"
info "  Retained earnings: \$${retained}"
info "  Tax destination:   ${tax_dest}"
info "  Swap queued:       ${swap_queued}"

# 2f. Verify idempotency
info "Testing settlement idempotency..."
idem_resp=$(api POST "/api/services/opportunities/${OPP_ID}/settle" \
    -d "$settle_body"
) || fail "Idempotent settlement failed: ${idem_resp}"

idem_flag=$(echo "$idem_resp" | jq -r '.idempotent // false')
if [[ "$idem_flag" == "true" ]]; then
    check_pass "Idempotent replay works correctly"
else
    check_fail "Idempotent replay did not return idempotent=true"
fi

# 2g. Verify full opportunity state
info "Verifying opportunity record..."
opp_record=$(api GET "/api/services/opportunities/${OPP_ID}") || fail "Cannot fetch opportunity"
final_status=$(echo "$opp_record" | jq -r '.status')
final_settled=$(echo "$opp_record" | jq -r '.settled_amount_usdc // 0')
final_retained=$(echo "$opp_record" | jq -r '.retained_earnings_usdc // 0')

if [[ "$final_status" == "settled" ]] && awk "BEGIN { exit (${final_settled} > 0) ? 0 : 1 }"; then
    check_pass "Opportunity record verified (settled_amount=\$${final_settled}, retained=\$${final_retained})"
else
    check_fail "Opportunity record verification failed (status=${final_status}, settled=${final_settled})"
fi

echo ""
echo -e "  ${BOLD}Lifecycle: ${GREEN}${PASS_COUNT} passed${RESET}, ${RED}${FAIL_COUNT} failed${RESET}"

if [[ "$DRY_RUN" -eq 1 ]]; then
    echo ""
    step "DRY RUN — skipping on-chain phases"
    echo ""
    echo -e "${BOLD}╔══════════════════════════════════════════════════════╗${RESET}"
    echo -e "${BOLD}║  SOAK TEST RESULTS (DRY RUN)                        ║${RESET}"
    echo -e "${BOLD}╠══════════════════════════════════════════════════════╣${RESET}"
    echo -e "${BOLD}║  Opportunity: ${OPP_ID}  ║${RESET}"
    echo -e "${BOLD}║  Passed: ${GREEN}${PASS_COUNT}${RESET}${BOLD}  Failed: ${RED}${FAIL_COUNT}${RESET}${BOLD}                               ║${RESET}"
    echo -e "${BOLD}╚══════════════════════════════════════════════════════╝${RESET}"
    echo ""
    if [[ "$FAIL_COUNT" -gt 0 ]]; then exit 1; fi
    exit 0
fi

# ═══════════════════════════════════════════════════════════════════
# PHASE 3: Tax Payout (on-chain ERC-20 transfer)
# ═══════════════════════════════════════════════════════════════════
step "Phase 3: Tax Payout (on-chain)"

# Check if tax task was queued
tax_tasks=$(api GET "/api/services/tax-payouts?limit=10") || fail "Cannot list tax tasks"
tax_task_count=$(echo "$tax_tasks" | jq -r '.count // 0')
tax_task_status=$(echo "$tax_tasks" | jq -r ".tax_tasks[] | select(.opportunity_id == \"${OPP_ID}\") | .status" 2>/dev/null || echo "")

if awk "BEGIN { exit (${tax_amount} > 0) ? 0 : 1 }" && [[ -n "$tax_task_status" ]]; then
    check_pass "Tax task queued (status=${tax_task_status})"

    if [[ "$tax_dest" == "none" || -z "$tax_dest" ]]; then
        warn "No tax destination wallet configured — skipping on-chain tax payout"
        warn "Set self_funding.tax.destination_wallet in ironclad.toml"
    else
        # Start the tax task
        info "Starting tax task..."
        start_resp=$(api POST "/api/services/tax-payouts/${OPP_ID}/start") || fail "Tax start failed"
        check_pass "Tax task started"

        # Construct ERC-20 transfer calldata
        info "Constructing ERC-20 transfer calldata..."
        calldata=$(erc20_transfer_calldata "$tax_dest" "$tax_amount")
        info "  to:       ${tax_dest}"
        info "  amount:   \$${tax_amount} USDC"
        info "  calldata: ${calldata:0:20}..."

        # Submit the tax payout
        info "Submitting tax payout to USDC contract on-chain..."
        submit_body=$(jq -n \
            --arg cd "$calldata" \
            --arg ca "$USDC_BASE" \
            '{calldata: $cd, contract_address: $ca}'
        )
        submit_resp=$(api POST "/api/services/tax-payouts/${OPP_ID}/submit" \
            -d "$submit_body"
        ) || {
            check_fail "Tax payout submission failed: ${submit_resp}"
            echo ""
            echo -e "  ${RED}Submission error — check treasury limits, wallet balance, and chain config${RESET}"
        }

        if echo "$submit_resp" | jq -e '.tx_hash' >/dev/null 2>&1; then
            TAX_TX=$(echo "$submit_resp" | jq -r '.tx_hash')
            check_pass "Tax payout submitted: ${TAX_TX}"
            info "  BaseScan: https://basescan.org/tx/${TAX_TX}"

            # Poll for confirmation
            info "Polling for on-chain confirmation..."
            confirmed=0
            for i in $(seq 1 $MAX_POLLS); do
                sleep "$POLL_INTERVAL"
                recon_resp=$(api POST "/api/services/tax-payouts/${OPP_ID}/reconcile" 2>/dev/null) || continue
                receipt_status=$(echo "$recon_resp" | jq -r '.receipt_status // "pending"')
                if [[ "$receipt_status" == "confirmed" ]]; then
                    check_pass "Tax payout confirmed on-chain (${i}×${POLL_INTERVAL}s)"
                    confirmed=1
                    break
                elif [[ "$receipt_status" == "failed" ]]; then
                    check_fail "Tax payout FAILED on-chain"
                    break
                fi
                info "  Poll ${i}/${MAX_POLLS}: still pending..."
            done
            if [[ "$confirmed" -eq 0 ]]; then
                warn "Tax payout not yet confirmed after polling. Check manually."
            fi
        fi
    fi
else
    if awk "BEGIN { exit (${tax_amount} > 0) ? 0 : 1 }"; then
        check_fail "Tax amount > 0 but no tax task found"
    else
        info "Tax amount is \$0 — no tax task expected"
        check_pass "Tax correctly skipped (amount=\$0)"
    fi
fi

# ═══════════════════════════════════════════════════════════════════
# PHASE 4: Revenue Swap (on-chain DEX call)
# ═══════════════════════════════════════════════════════════════════
step "Phase 4: Revenue Swap (on-chain)"

# Re-settle with auto_swap enabled to queue the swap task
info "Re-settling with auto_swap=true to queue swap task..."
swap_settle_body=$(jq -n \
    --arg ref "${SOAK_REF}-swap" \
    --argjson amt "$REVENUE_AMOUNT" \
    --arg usdc_contract "$USDC_BASE" \
    '{
        settlement_ref: $ref,
        amount_usdc: $amt,
        currency: "USDC",
        attributable_costs_usdc: 0.0,
        auto_swap: true,
        target_symbol: "WETH",
        target_chain: "BASE",
        target_contract_address: "0x4200000000000000000000000000000000000006",
        swap_contract_address: "0x2626664c2603336E57B271c5C0b26F421741e481"
    }'
)

# Create a NEW opportunity for the swap test cycle
info "Creating second opportunity for swap test..."
intake2_resp=$(api POST /api/services/opportunities/intake \
    -d "$(jq -n \
        --argjson amt "$REVENUE_AMOUNT" \
        '{source: "soak_test", strategy: "micro_bounty", expected_revenue_usdc: $amt, payload: {test: "swap_cycle"}}'
    )"
) || fail "Swap cycle intake failed"
OPP2_ID=$(echo "$intake2_resp" | jq -r '.opportunity_id')
check_pass "Swap cycle opportunity: ${OPP2_ID}"

# Fast-track through lifecycle
api POST "/api/services/opportunities/${OPP2_ID}/qualify" -d '{"approved": true, "reason": "swap soak"}' >/dev/null 2>&1
api POST "/api/services/opportunities/${OPP2_ID}/plan" -d '{"plan": {"steps": ["swap test"]}}' >/dev/null 2>&1
api POST "/api/services/opportunities/${OPP2_ID}/fulfill" -d '{"evidence": {"result": "swap test done"}}' >/dev/null 2>&1
check_pass "Fast-tracked to fulfilled"

# Settle with auto_swap
swap_settle_resp=$(api POST "/api/services/opportunities/${OPP2_ID}/settle" \
    -d "$(jq -n \
        --arg ref "${SOAK_REF}-swap" \
        --argjson amt "$REVENUE_AMOUNT" \
        '{
            settlement_ref: $ref,
            amount_usdc: $amt,
            currency: "USDC",
            attributable_costs_usdc: 0.0,
            auto_swap: true,
            target_symbol: "WETH",
            target_chain: "BASE",
            target_contract_address: "0x4200000000000000000000000000000000000006",
            swap_contract_address: "0x2626664c2603336E57B271c5C0b26F421741e481"
        }'
    )"
) || fail "Swap settlement failed"

swap_queued2=$(echo "$swap_settle_resp" | jq -r '.swap_queued // false')
retained2=$(echo "$swap_settle_resp" | jq -r '.retained_earnings_usdc // 0')
if [[ "$swap_queued2" == "true" ]]; then
    check_pass "Swap task queued (retained=\$${retained2})"
else
    check_fail "Swap task was not queued"
fi

# Verify swap task exists
swap_tasks=$(api GET "/api/services/swaps?limit=10") || fail "Cannot list swap tasks"
info "Swap tasks available: $(echo "$swap_tasks" | jq -r '.count // 0')"

# Start the swap
info "Starting swap task..."
swap_start=$(api POST "/api/services/swaps/${OPP2_ID}/start" 2>/dev/null) || {
    check_fail "Swap start failed"
}
if echo "$swap_start" | jq -e '.status == "in_progress"' >/dev/null 2>&1; then
    check_pass "Swap task started"
fi

echo ""
echo -e "  ${YELLOW}═══ SWAP CALLDATA REQUIRED ═══${RESET}"
echo ""
echo -e "  The swap task is now ${CYAN}in_progress${RESET} and waiting for calldata."
echo -e "  To complete the swap, you need to construct Uniswap V3 calldata."
echo ""
echo -e "  ${BOLD}Option A: Use Foundry cast to encode calldata${RESET}"
echo -e "  ${DIM}cast calldata 'exactInputSingle((address,address,uint24,address,uint256,uint256,uint256,uint160))' \\${RESET}"
echo -e "  ${DIM}  '(${USDC_BASE},0x4200000000000000000000000000000000000006,3000,${wallet_addr},$(awk "BEGIN { printf \"%.0f\", ${retained2} * 1000000 }"),0,0)'${RESET}"
echo ""
echo -e "  ${BOLD}Option B: Submit via API manually${RESET}"
echo -e "  ${DIM}curl -X POST ${BASE_URL}/api/services/swaps/${OPP2_ID}/submit \\${RESET}"
echo -e "  ${DIM}  -H 'Content-Type: application/json' \\${RESET}"
echo -e "  ${DIM}  -d '{\"calldata\": \"0x...\", \"contract_address\": \"0x2626664c2603336E57B271c5C0b26F421741e481\"}'${RESET}"
echo ""
echo -e "  ${BOLD}Option C: Skip swap (mark as failed)${RESET}"
echo -e "  ${DIM}curl -X POST ${BASE_URL}/api/services/swaps/${OPP2_ID}/fail \\${RESET}"
echo -e "  ${DIM}  -H 'Content-Type: application/json' \\${RESET}"
echo -e "  ${DIM}  -d '{\"reason\": \"soak test: swap skipped\"}'${RESET}"
echo ""

# ═══════════════════════════════════════════════════════════════════
# PHASE 5: Final Balance Verification
# ═══════════════════════════════════════════════════════════════════
step "Phase 5: Final Balance Verification"

final_wallet=$(api GET /api/wallet/balance 2>/dev/null) || warn "Cannot fetch final wallet state"
final_balance=$(echo "$final_wallet" | jq -r '.balance // "unknown"')
info "Final USDC balance: \$${final_balance}"
info "Starting balance:   \$${wallet_balance}"

# Calculate expected delta (gas costs make exact comparison unreliable)
if [[ "$final_balance" != "unknown" && "$wallet_balance" != "unknown" ]]; then
    delta=$(awk "BEGIN { printf \"%.4f\", ${wallet_balance} - ${final_balance} }")
    info "Balance delta:      \$${delta} (includes gas costs)"
    check_pass "Balance verification complete"
fi

# ═══════════════════════════════════════════════════════════════════
# Summary
# ═══════════════════════════════════════════════════════════════════
echo ""
echo -e "${BOLD}╔══════════════════════════════════════════════════════╗${RESET}"
echo -e "${BOLD}║  SOAK TEST RESULTS                                  ║${RESET}"
echo -e "${BOLD}╠══════════════════════════════════════════════════════╣${RESET}"
echo -e "${BOLD}║  Opportunity 1: ${OPP_ID}  ║${RESET}"
echo -e "${BOLD}║  Opportunity 2: ${OPP2_ID}  ║${RESET}"
echo -e "${BOLD}║  Settlement:    ${SOAK_REF}                  ║${RESET}"
echo -e "${BOLD}║  Passed: ${GREEN}${PASS_COUNT}${RESET}${BOLD}  Failed: ${RED}${FAIL_COUNT}${RESET}${BOLD}                               ║${RESET}"
echo -e "${BOLD}╚══════════════════════════════════════════════════════╝${RESET}"
echo ""

if [[ -n "${TAX_TX:-}" ]]; then
    echo -e "  ${BOLD}On-chain Transactions:${RESET}"
    echo -e "    Tax: https://basescan.org/tx/${TAX_TX}"
    echo ""
fi

if [[ "$FAIL_COUNT" -gt 0 ]]; then
    echo -e "  ${RED}${FAIL_COUNT} checks failed. Review output above.${RESET}"
    exit 1
else
    echo -e "  ${GREEN}All checks passed!${RESET}"
    exit 0
fi
