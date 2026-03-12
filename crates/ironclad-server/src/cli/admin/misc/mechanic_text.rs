include!("mechanic_text_local.rs");
include!("mechanic_text_gateway.rs");
include!("mechanic_text_security.rs");

pub async fn cmd_mechanic(
    base_url: &str,
    repair: bool,
    json_output: bool,
    allow_jobs: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    if json_output {
        return cmd_mechanic_json(base_url, repair, allow_jobs).await;
    }
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    println!(
        "\n  {BOLD}Ironclad Mechanic{RESET}{}\n",
        if repair { " (--repair mode)" } else { "" }
    );

    let ironclad_dir = ironclad_core::home_dir().join(".ironclad");
    let mut fixed = 0u32;

    run_mechanic_text_local_preflight(&ironclad_dir, repair, &mut fixed)?;
    run_mechanic_text_gateway_checks(base_url, &ironclad_dir, repair, allow_jobs, &mut fixed).await?;
    run_mechanic_text_security_and_finalize(&ironclad_dir, repair, &mut fixed)?;
    Ok(())
}
