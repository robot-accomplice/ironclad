#!/usr/bin/env python3
"""
One-time migration: Legacy PostgreSQL → Ironclad SQLite.

Most Legacy instances don't have a PostgreSQL database.  Ours does (two of
them: `mentat` and `agentic_bot`).  This script pulls everything worth keeping
into Ironclad's state.db so nothing is lost when we retire the PG databases.

Usage:
    python3 tools/migrate-legacy-pg.py                # dry-run (prints what it would do)
    python3 tools/migrate-legacy-pg.py --commit       # actually write to SQLite
    python3 tools/migrate-legacy-pg.py --commit --db ~/custom/state.db

Requires: psycopg2 (pip install psycopg2-binary)
"""

from __future__ import annotations

import argparse
import json
import os
import sqlite3
import sys
import uuid
from datetime import datetime, timezone
from pathlib import Path

try:
    import psycopg2
    import psycopg2.extras
except ImportError:
    print("psycopg2 not installed.  Run:  pip install psycopg2-binary", file=sys.stderr)
    sys.exit(1)


# ── Constants ──────────────────────────────────────────────────────────

IRONCLAD_DB = Path.home() / ".ironclad" / "state.db"

SENSITIVE_KEYS: set[str] = set()  # import everything, including credentials

# ── Helpers ────────────────────────────────────────────────────────────

class Stats:
    def __init__(self):
        self.inserted = 0
        self.skipped = 0
        self.warnings: list[str] = []

    def __repr__(self):
        return f"inserted={self.inserted} skipped={self.skipped} warnings={len(self.warnings)}"


def ts_now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def is_sensitive(key: str) -> bool:
    lower = key.lower()
    return any(s in lower for s in SENSITIVE_KEYS)


def pg_connect(dbname: str, user: str | None = None) -> psycopg2.extensions.connection:
    kwargs = {"dbname": dbname}
    if user:
        kwargs["user"] = user
    return psycopg2.connect(**kwargs)


def safe_json(val) -> str:
    if val is None:
        return "{}"
    if isinstance(val, str):
        return val
    return json.dumps(val, default=str)


def fmt_ts(val) -> str:
    if val is None:
        return ts_now()
    if hasattr(val, "isoformat"):
        return val.isoformat()
    return str(val)


# ── Mentat importers ──────────────────────────────────────────────────

def import_memories(pg, sl, stats: Stats):
    """mentat.memories → semantic_memory (category/key/value model)"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("""
        SELECT memory_id, category, content, importance, tags, metadata,
               created_at, updated_at
        FROM memories ORDER BY id
    """)
    for row in cur:
        mid = str(row["memory_id"])
        cat = row["category"] or "general"
        content = row["content"] or ""
        importance = row["importance"] or 5
        tags = row["tags"] or []
        meta = row["metadata"] or {}
        created = fmt_ts(row["created_at"])
        updated = fmt_ts(row["updated_at"])

        key = f"mem:{mid}"
        value = json.dumps({
            "content": content,
            "importance": importance,
            "tags": tags,
            "metadata": meta,
        }, default=str)

        try:
            sl.execute(
                """INSERT OR IGNORE INTO semantic_memory
                   (id, category, key, value, confidence, created_at, updated_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?)""",
                (mid, cat, key, value, min(importance / 10.0, 1.0), created, updated),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"memories {mid}: {e}")


def import_key_value(pg, sl, stats: Stats):
    """mentat.key_value → identity (key/value store)"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("SELECT key, value, updated_at FROM key_value ORDER BY key")
    for row in cur:
        key = row["key"]
        if is_sensitive(key):
            stats.skipped += 1
            stats.warnings.append(f"skipped sensitive: {key}")
            continue
        value = safe_json(row["value"])
        try:
            sl.execute(
                "INSERT OR REPLACE INTO identity (key, value) VALUES (?, ?)",
                (f"pg:kv:{key}", value),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"key_value {key}: {e}")


def import_mentat_tasks(pg, sl, stats: Stats):
    """mentat.tasks → tasks"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("""
        SELECT task_id, title, description, status, priority,
               result, metadata, details, created_at, completed_at
        FROM tasks ORDER BY id
    """)
    for row in cur:
        tid = str(row["task_id"])
        title = row["title"] or "(untitled)"
        desc = row["description"]
        status = row["status"] or "pending"
        priority = row["priority"] or 0
        source = json.dumps({
            "origin": "pg:mentat:tasks",
            "result": row["result"],
            "metadata": row["metadata"],
            "details": row["details"],
            "completed_at": fmt_ts(row["completed_at"]) if row["completed_at"] else None,
        }, default=str)
        created = fmt_ts(row["created_at"])

        try:
            sl.execute(
                """INSERT OR IGNORE INTO tasks
                   (id, title, description, status, priority, source, created_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?)""",
                (tid, title, desc, status, priority, source, created),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"task {tid}: {e}")


def import_transactions(pg, sl, stats: Stats):
    """mentat.transactions → transactions"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("""
        SELECT transaction_id, type, category, amount::float8,
               description, status, metadata, created_at
        FROM transactions ORDER BY id
    """)
    for row in cur:
        tid = str(row["transaction_id"])
        tx_type = row["type"]
        amount = float(row["amount"] or 0)
        meta = json.dumps({
            "source": "pg:mentat:transactions",
            "category": row["category"],
            "status": row["status"],
            "metadata": row["metadata"],
        }, default=str)
        created = fmt_ts(row["created_at"])

        try:
            sl.execute(
                """INSERT OR IGNORE INTO transactions
                   (id, tx_type, amount, currency, metadata_json, created_at)
                   VALUES (?, ?, ?, 'USD', ?, ?)""",
                (tid, tx_type, amount, meta, created),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"transaction {tid}: {e}")


def import_cost_snapshots(pg, sl, stats: Stats):
    """mentat.cost_snapshots → metric_snapshots (NOT inference_costs).

    These are cumulative point-in-time totals, not per-call records.
    Importing them into inference_costs and summing produces wildly
    inflated figures, so we store them as metric snapshots instead.
    """
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("""
        SELECT captured_at, snapshot_type, actual_cost::float8,
               total_input_tokens, total_output_tokens,
               by_tier, by_agent
        FROM cost_snapshots ORDER BY id
    """)
    for row in cur:
        cid = str(uuid.uuid4())
        metrics = json.dumps({
            "source": "pg:mentat:cost_snapshots",
            "snapshot_type": row["snapshot_type"],
            "actual_cost": float(row["actual_cost"] or 0),
            "total_input_tokens": row["total_input_tokens"] or 0,
            "total_output_tokens": row["total_output_tokens"] or 0,
            "by_tier": row["by_tier"],
            "by_agent": row["by_agent"],
        }, default=str)
        created = fmt_ts(row["captured_at"])

        try:
            sl.execute(
                """INSERT OR IGNORE INTO metric_snapshots
                   (id, metrics_json, created_at)
                   VALUES (?, ?, ?)""",
                (cid, metrics, created),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"cost_snapshot: {e}")


def import_specialist_reports(pg, sl, stats: Stats):
    """mentat.specialist_reports → episodic_memory"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("""
        SELECT report_id, specialist, trigger_type, status,
               summary, full_output, actionable, created_at
        FROM specialist_reports ORDER BY id
    """)
    for row in cur:
        rid = str(row["report_id"])
        specialist = row["specialist"]
        trigger = row["trigger_type"]
        content = row["summary"] or row["full_output"] or "(empty report)"
        importance = 8 if row["actionable"] else 5
        created = fmt_ts(row["created_at"])

        try:
            sl.execute(
                """INSERT OR IGNORE INTO episodic_memory
                   (id, classification, content, importance, created_at)
                   VALUES (?, ?, ?, ?, ?)""",
                (rid, f"specialist:{specialist}:{trigger}", content, importance, created),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"specialist_report {rid}: {e}")


def import_context_snapshots(pg, sl, stats: Stats):
    """mentat.context_snapshots → episodic_memory"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("""
        SELECT id, snapshot_type, content, trigger_reason, created_at
        FROM context_snapshots ORDER BY id
    """)
    for row in cur:
        sid = f"pg:ctx:{row['id']}"
        snap_type = row["snapshot_type"]
        trigger = row["trigger_reason"] or "unknown"
        content = f"[context_snapshot:{snap_type}] trigger={trigger}\n{safe_json(row['content'])}"
        created = fmt_ts(row["created_at"])

        try:
            sl.execute(
                """INSERT OR IGNORE INTO episodic_memory
                   (id, classification, content, importance, created_at)
                   VALUES (?, ?, ?, ?, ?)""",
                (sid, f"context_snapshot:{snap_type}", content, 6, created),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"context_snapshot {sid}: {e}")


def import_maintenance_log(pg, sl, stats: Stats):
    """mentat.maintenance_log → episodic_memory (low importance)"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("""
        SELECT id, check_name, category, status, details, fix_applied, run_at
        FROM maintenance_log ORDER BY id
    """)
    for row in cur:
        mid = f"pg:maint:{row['id']}"
        cat = row["category"]
        check = row["check_name"]
        status = row["status"]
        details = row["details"] or ""
        fix = row["fix_applied"] or False
        created = fmt_ts(row["run_at"])

        content = f"[maintenance:{cat}:{check}] {status}: {details}\nFix applied: {fix}"

        try:
            sl.execute(
                """INSERT OR IGNORE INTO episodic_memory
                   (id, classification, content, importance, created_at)
                   VALUES (?, ?, ?, ?, ?)""",
                (mid, f"maintenance:{cat}", content, 2, created),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"maintenance_log {mid}: {e}")


def import_session_analytics(pg, sl, stats: Stats):
    """mentat.session_analytics → metric_snapshots"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("""
        SELECT id, session_id, file_size_bytes, line_count,
               age_hours::float8, is_active, metadata, captured_at
        FROM session_analytics ORDER BY id
    """)
    for row in cur:
        sid = f"pg:analytics:{row['id']}"
        metrics = json.dumps({
            "source": "pg:mentat:session_analytics",
            "session_id": row["session_id"],
            "file_size_bytes": row["file_size_bytes"],
            "line_count": row["line_count"],
            "age_hours": row["age_hours"],
            "is_active": row["is_active"],
            "metadata": row["metadata"],
        }, default=str)
        created = fmt_ts(row["captured_at"])

        try:
            sl.execute(
                """INSERT OR IGNORE INTO metric_snapshots
                   (id, metrics_json, created_at)
                   VALUES (?, ?, ?)""",
                (sid, metrics, created),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"session_analytics {sid}: {e}")


def import_knowledge_tables(pg, sl, stats: Stats):
    """
    mentat.{bounty_knowledge, exploit_patterns, novel_patterns,
            audit_findings, audit_targets, audit_reading_list}
    → semantic_memory
    """
    tables = [
        ("bounty_knowledge",
         "SELECT id, category, protocol, vulnerability_type, description, result FROM bounty_knowledge",
         lambda r: (f"pg:bounty:{r['id']}", "security",
                    f"bounty:{r['category']}:{r.get('vulnerability_type','?')}",
                    json.dumps(dict(r), default=str), 0.7)),
        ("exploit_patterns",
         "SELECT id, pattern_name, category, description, detection_regex FROM exploit_patterns",
         lambda r: (f"pg:exploit:{r['id']}", "security",
                    f"exploit:{r['pattern_name']}",
                    json.dumps(dict(r), default=str), 0.8)),
        ("novel_patterns",
         "SELECT id, pattern_name, category, protocol, description, why_novel, tradeoffs FROM novel_patterns",
         lambda r: (f"pg:novel:{r['id']}", "research",
                    f"novel:{r['pattern_name']}",
                    json.dumps(dict(r), default=str), 0.6)),
        ("audit_findings",
         "SELECT id, protocol, finding_type, severity, status, notes FROM audit_findings",
         lambda r: (f"pg:audit_finding:{r['id']}", "security",
                    f"finding:{r['protocol']}:{r['finding_type']}",
                    json.dumps(dict(r), default=str), 0.9)),
        ("audit_targets",
         "SELECT id, protocol, protocol_type, chain, status, key_learnings FROM audit_targets",
         lambda r: (f"pg:audit_target:{r['id']}", "security",
                    f"target:{r['protocol']}",
                    json.dumps(dict(r), default=str), 0.7)),
        ("audit_reading_list",
         "SELECT id, source, title, url, key_lesson, read_status FROM audit_reading_list",
         lambda r: (f"pg:reading:{r['id']}", "research",
                    f"reading:{r.get('title','?')[:80]}",
                    json.dumps(dict(r), default=str), 0.5)),
    ]

    for table_name, query, mapper in tables:
        cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
        try:
            cur.execute(query)
        except psycopg2.Error as e:
            stats.warnings.append(f"{table_name}: {e}")
            continue

        for row in cur:
            row_id, cat, key, value, confidence = mapper(row)
            now = ts_now()
            try:
                sl.execute(
                    """INSERT OR IGNORE INTO semantic_memory
                       (id, category, key, value, confidence, created_at, updated_at)
                       VALUES (?, ?, ?, ?, ?, ?, ?)""",
                    (row_id, cat, key, value, confidence, now, now),
                )
                stats.inserted += 1
            except sqlite3.Error as e:
                stats.warnings.append(f"{table_name} {row_id}: {e}")


def import_dune_quotes(pg, sl, stats: Stats):
    """mentat.dune_quotes → semantic_memory (personality category)"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("SELECT id, quote, character, book FROM dune_quotes ORDER BY id")
    for row in cur:
        qid = f"pg:quote:{row['id']}"
        char = row["character"] or "Unknown"
        book = row["book"] or "Dune"
        key = f"quote:{row['id']}"
        value = json.dumps({
            "quote": row["quote"],
            "character": char,
            "book": book,
        })
        now = ts_now()

        try:
            sl.execute(
                """INSERT OR IGNORE INTO semantic_memory
                   (id, category, key, value, confidence, created_at, updated_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?)""",
                (qid, "personality", key, value, 0.3, now, now),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"dune_quote {qid}: {e}")


def import_conway_audit_log(pg, sl, stats: Stats):
    """mentat.conway_audit_log → episodic_memory"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("""
        SELECT id, timestamp, tool_name, network, parameters,
               result, cost_usdc, approved_by, blocked, block_reason
        FROM conway_audit_log ORDER BY id
    """)
    for row in cur:
        cid = f"pg:conway:{row['id']}"
        tool = row["tool_name"]
        network = row["network"]
        blocked = row["blocked"] or False
        content = json.dumps({
            "tool": tool,
            "network": network,
            "parameters": row["parameters"],
            "result": row["result"],
            "cost_usdc": float(row["cost_usdc"] or 0),
            "approved_by": row["approved_by"],
            "blocked": blocked,
            "block_reason": row["block_reason"],
        }, default=str)
        created = fmt_ts(row["timestamp"])
        importance = 7 if blocked else 3

        try:
            sl.execute(
                """INSERT OR IGNORE INTO episodic_memory
                   (id, classification, content, importance, created_at)
                   VALUES (?, ?, ?, ?, ?)""",
                (cid, f"conway:{tool}", content, importance, created),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"conway_audit_log {cid}: {e}")


def import_protocol_analysis(pg, sl, stats: Stats):
    """mentat.protocol_analysis → semantic_memory"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("""
        SELECT id, protocol, chain, contract_address, analysis_status,
               contract_type, risk_areas, notes
        FROM protocol_analysis ORDER BY id
    """)
    now = ts_now()
    for row in cur:
        pid = f"pg:protocol:{row['id']}"
        protocol = row["protocol"]
        value = json.dumps(dict(row), default=str)

        try:
            sl.execute(
                """INSERT OR IGNORE INTO semantic_memory
                   (id, category, key, value, confidence, created_at, updated_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?)""",
                (pid, "security", f"protocol:{protocol}", value, 0.7, now, now),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"protocol_analysis {pid}: {e}")


# ── Agentic Bot importers ─────────────────────────────────────────────

def import_bot_memory(pg, sl, stats: Stats):
    """agentic_bot.bot_memory → semantic_memory"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("SELECT id, memory_type, key, value, confidence, source FROM bot_memory ORDER BY id")
    now = ts_now()
    for row in cur:
        bid = f"pg:bot_mem:{row['id']}"
        mem_type = row["memory_type"]
        key = row["key"]
        value = safe_json(row["value"])
        conf = row["confidence"] or 0.5

        try:
            sl.execute(
                """INSERT OR IGNORE INTO semantic_memory
                   (id, category, key, value, confidence, created_at, updated_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?)""",
                (bid, mem_type, f"bot:{key}", value, conf, now, now),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"bot_memory {bid}: {e}")


def import_bot_tasks(pg, sl, stats: Stats):
    """agentic_bot.tasks → tasks"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("""
        SELECT id, task_type, description, priority, status, created_at
        FROM tasks ORDER BY id
    """)
    for row in cur:
        tid = f"pg:bot_task:{row['id']}"
        desc = row["description"]
        status = row["status"] or "pending"
        priority = row["priority"] or 0
        created = fmt_ts(row["created_at"])

        try:
            sl.execute(
                """INSERT OR IGNORE INTO tasks
                   (id, title, description, status, priority, source, created_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?)""",
                (tid, desc[:120], desc, status, priority, "pg:agentic_bot:tasks", created),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"bot task {tid}: {e}")


def import_bot_strategies(pg, sl, stats: Stats):
    """agentic_bot.self_funding_strategies → semantic_memory"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("""
        SELECT id, strategy_name, strategy_type, is_active,
               total_revenue, roi, config
        FROM self_funding_strategies ORDER BY id
    """)
    now = ts_now()
    for row in cur:
        sid = f"pg:strategy:{row['id']}"
        name = row["strategy_name"]
        value = json.dumps(dict(row), default=str)

        try:
            sl.execute(
                """INSERT OR IGNORE INTO semantic_memory
                   (id, category, key, value, confidence, created_at, updated_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?)""",
                (sid, "financial", f"strategy:{name}", value, 0.7, now, now),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"strategy {sid}: {e}")


def import_bot_users(pg, sl, stats: Stats):
    """agentic_bot.users → identity"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("""
        SELECT user_id, display_name, telegram_chat_id, signal_phone,
               last_interface, preferences
        FROM users ORDER BY id
    """)
    for row in cur:
        uid = row["user_id"]
        value = json.dumps({
            "display_name": row["display_name"],
            "telegram_chat_id": row["telegram_chat_id"],
            "signal_phone": row["signal_phone"],
            "last_interface": row["last_interface"],
            "preferences": row["preferences"],
        }, default=str)

        try:
            sl.execute(
                "INSERT OR REPLACE INTO identity (key, value) VALUES (?, ?)",
                (f"pg:user:{uid}", value),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"user {uid}: {e}")


def import_bot_task_patterns(pg, sl, stats: Stats):
    """agentic_bot.task_patterns → procedural_memory"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("""
        SELECT id, task_type, successful_tools, failed_tools,
               success_count, failure_count, confidence
        FROM task_patterns ORDER BY id
    """)
    now = ts_now()
    for row in cur:
        pid = f"pg:pattern:{row['id']}"
        name = row["task_type"]
        steps = json.dumps({
            "successful_tools": row["successful_tools"],
            "failed_tools": row["failed_tools"],
        }, default=str)

        try:
            sl.execute(
                """INSERT OR IGNORE INTO procedural_memory
                   (id, name, steps, success_count, failure_count, created_at, updated_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?)""",
                (pid, f"bot:{name}", steps,
                 row["success_count"] or 0, row["failure_count"] or 0, now, now),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"task_pattern {pid}: {e}")


def import_bot_personality(pg, sl, stats: Stats):
    """agentic_bot.personality_feedback + personality_state → episodic_memory"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    try:
        cur.execute("SELECT id, timestamp, feedback_type, feedback_signal, analysis FROM personality_feedback")
        for row in cur:
            fid = f"pg:personality:{row['id']}"
            content = json.dumps(dict(row), default=str)
            created = fmt_ts(row["timestamp"])
            try:
                sl.execute(
                    """INSERT OR IGNORE INTO episodic_memory
                       (id, classification, content, importance, created_at)
                       VALUES (?, ?, ?, ?, ?)""",
                    (fid, "personality_feedback", content, 4, created),
                )
                stats.inserted += 1
            except sqlite3.Error as e:
                stats.warnings.append(f"personality_feedback {fid}: {e}")
    except psycopg2.Error as e:
        stats.warnings.append(f"personality_feedback: {e}")


# ── DB index (self-referential schema catalog) ────────────────────────

def import_db_index(pg, sl, stats: Stats):
    """mentat.db_index → identity (preserves Duncan's schema self-knowledge)"""
    cur = pg.cursor(cursor_factory=psycopg2.extras.DictCursor)
    cur.execute("SELECT table_name, description, key_columns, typical_queries, notes FROM db_index")
    for row in cur:
        table = row["table_name"]
        value = json.dumps(dict(row), default=str)
        try:
            sl.execute(
                "INSERT OR REPLACE INTO identity (key, value) VALUES (?, ?)",
                (f"pg:schema:{table}", value),
            )
            stats.inserted += 1
        except sqlite3.Error as e:
            stats.warnings.append(f"db_index {table}: {e}")


# ── Orchestrator ──────────────────────────────────────────────────────

def run(db_path: Path, dry_run: bool):
    print(f"\n  ╭─ Legacy PostgreSQL → Ironclad SQLite ──────────")
    print(f"  │ Target: {db_path}")
    print(f"  │ Mode:   {'DRY RUN (no writes)' if dry_run else 'COMMIT'}")
    print(f"  │")

    sl = sqlite3.connect(str(db_path))
    sl.execute("PRAGMA journal_mode=WAL")
    sl.execute("PRAGMA foreign_keys=ON")

    all_stats: dict[str, Stats] = {}

    # ── mentat ────────────────────────────────────────────
    print("  │ Connecting to mentat ... ", end="", flush=True)
    try:
        pg_mentat = pg_connect("mentat")
        print("✔")

        importers_mentat = [
            ("memories",           import_memories),
            ("key_value",          import_key_value),
            ("tasks",              import_mentat_tasks),
            ("transactions",       import_transactions),
            ("cost_snapshots",     import_cost_snapshots),
            ("specialist_reports", import_specialist_reports),
            ("context_snapshots",  import_context_snapshots),
            ("maintenance_log",    import_maintenance_log),
            ("session_analytics",  import_session_analytics),
            ("knowledge_tables",   import_knowledge_tables),
            ("dune_quotes",        import_dune_quotes),
            ("conway_audit_log",   import_conway_audit_log),
            ("protocol_analysis",  import_protocol_analysis),
            ("db_index",           import_db_index),
        ]

        for name, fn in importers_mentat:
            stats = Stats()
            try:
                fn(pg_mentat, sl, stats)
            except Exception as e:
                stats.warnings.append(f"FATAL: {e}")
            all_stats[f"mentat.{name}"] = stats
            icon = "✔" if not stats.warnings else "⚠"
            print(f"  │ {icon} {name:<22} {stats.inserted:>5} rows")

        pg_mentat.close()
    except psycopg2.Error as e:
        print(f"✘ ({e})")

    # ── agentic_bot ───────────────────────────────────────
    print("  │")
    print("  │ Connecting to agentic_bot ... ", end="", flush=True)
    try:
        pg_bot = pg_connect("agentic_bot", user="bot_user")
        print("✔")

        importers_bot = [
            ("bot_memory",      import_bot_memory),
            ("tasks",           import_bot_tasks),
            ("strategies",      import_bot_strategies),
            ("users",           import_bot_users),
            ("task_patterns",   import_bot_task_patterns),
            ("personality",     import_bot_personality),
        ]

        for name, fn in importers_bot:
            stats = Stats()
            try:
                fn(pg_bot, sl, stats)
            except Exception as e:
                stats.warnings.append(f"FATAL: {e}")
            all_stats[f"agentic_bot.{name}"] = stats
            icon = "✔" if not stats.warnings else "⚠"
            print(f"  │ {icon} {name:<22} {stats.inserted:>5} rows")

        pg_bot.close()
    except psycopg2.Error as e:
        print(f"✘ ({e})")
        print("  │   (legacy database, non-critical)")

    # ── Summary ───────────────────────────────────────────
    total_inserted = sum(s.inserted for s in all_stats.values())
    total_skipped = sum(s.skipped for s in all_stats.values())
    total_warnings = sum(len(s.warnings) for s in all_stats.values())

    print("  │")
    print(f"  │ Total: {total_inserted} inserted, {total_skipped} skipped, {total_warnings} warnings")

    if total_warnings > 0:
        print("  │")
        print("  │ Warnings:")
        for area, stats in all_stats.items():
            for w in stats.warnings:
                print(f"  │   ⚠ {area}: {w}")

    if dry_run:
        print("  │")
        print("  │ DRY RUN — rolling back all changes")
        sl.rollback()
    else:
        sl.commit()
        print("  │")
        print(f"  │ ✔ Committed to {db_path}")

    sl.close()
    print("  ╰──────────────────────────────────────────────────")
    print()


def main():
    parser = argparse.ArgumentParser(
        description="One-time migration: Legacy PostgreSQL → Ironclad SQLite"
    )
    parser.add_argument(
        "--commit", action="store_true",
        help="Actually write to SQLite (default is dry-run)",
    )
    parser.add_argument(
        "--db", type=Path, default=IRONCLAD_DB,
        help=f"Path to Ironclad state.db (default: {IRONCLAD_DB})",
    )
    args = parser.parse_args()

    if not args.db.exists():
        print(f"ERROR: {args.db} does not exist.  Run `ironclad serve` once first.", file=sys.stderr)
        sys.exit(1)

    run(args.db, dry_run=not args.commit)


if __name__ == "__main__":
    main()
