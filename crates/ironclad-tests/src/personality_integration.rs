use ironclad_core::personality::{self, OsVoice};
use ironclad_server::PersonalityState;

#[test]
fn personality_state_from_workspace_loads_defaults() {
    let dir = tempfile::tempdir().unwrap();
    personality::write_defaults(dir.path()).unwrap();

    let state = PersonalityState::from_workspace(dir.path());
    assert!(!state.soul_text.is_empty());
    assert!(state.soul_text.contains("Roboticus"));
    assert_eq!(state.identity.name, "Roboticus");
    assert_eq!(state.identity.generated_by, "default");
    assert!(!state.firmware_text.is_empty());
    assert!(state.firmware_text.contains("YOU MUST"));
}

#[test]
fn personality_state_empty_has_no_text() {
    let state = PersonalityState::empty();
    assert!(state.soul_text.is_empty());
    assert!(state.firmware_text.is_empty());
    assert!(state.identity.name.is_empty());
    assert_eq!(state.identity.generated_by, "none");
}

#[test]
fn personality_state_from_empty_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let state = PersonalityState::from_workspace(dir.path());
    assert!(state.soul_text.is_empty());
    assert!(state.firmware_text.is_empty());
}

#[test]
fn personality_state_separates_soul_and_firmware() {
    let dir = tempfile::tempdir().unwrap();
    personality::write_defaults(dir.path()).unwrap();

    let state = PersonalityState::from_workspace(dir.path());
    assert!(state.soul_text.contains("iron-plated"));
    assert!(!state.soul_text.contains("YOU MUST"));
    assert!(state.firmware_text.contains("YOU MUST"));
    assert!(!state.firmware_text.contains("iron-plated"));
}

#[test]
fn personality_reload_picks_up_changes() {
    let dir = tempfile::tempdir().unwrap();
    personality::write_defaults(dir.path()).unwrap();

    let state1 = PersonalityState::from_workspace(dir.path());
    assert_eq!(state1.identity.name, "Roboticus");

    let custom_os = personality::generate_os_toml("CustomBot", "formal", "wait", "research");
    std::fs::write(dir.path().join("OS.toml"), &custom_os).unwrap();

    let state2 = PersonalityState::from_workspace(dir.path());
    assert_eq!(state2.identity.name, "CustomBot");
    assert_eq!(state2.identity.generated_by, "short-interview");
    assert_eq!(state2.voice.formality, "formal");
}

#[test]
fn voice_summary_in_soul_text_when_non_default() {
    let dir = tempfile::tempdir().unwrap();
    let custom_os = personality::generate_os_toml("TestBot", "casual", "initiative", "developer");
    std::fs::write(dir.path().join("OS.toml"), &custom_os).unwrap();

    let state = PersonalityState::from_workspace(dir.path());
    assert!(state.soul_text.contains("## Voice Profile"));
    assert!(state.soul_text.contains("Formality: casual"));
    assert!(state.soul_text.contains("Proactiveness: initiative"));
    assert!(state.soul_text.contains("Domain: developer"));
}

#[test]
fn voice_summary_short_in_status_format() {
    let voice = OsVoice {
        formality: "formal".into(),
        proactiveness: "wait".into(),
        verbosity: "verbose".into(),
        humor: "none".into(),
        domain: "research".into(),
    };
    let short = personality::voice_summary_short(&voice);
    assert!(short.contains("formal"));
    assert!(short.contains("wait"));
    assert!(short.contains("verbose"));
    assert!(short.contains("none"));
    assert!(short.contains("research"));
}
