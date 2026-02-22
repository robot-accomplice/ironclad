use ironclad_agent::agent_loop::{AgentLoop, ReactAction, ReactState};

#[test]
fn react_state_initial_is_idle() {
    let agent = AgentLoop::new(10);
    assert_eq!(agent.state, ReactState::Idle);
}

#[test]
fn think_transitions_to_thinking() {
    let mut agent = AgentLoop::new(10);
    let state = agent.transition(ReactAction::Think);
    assert_eq!(state, ReactState::Thinking);
}

#[test]
fn act_transitions_to_acting() {
    let mut agent = AgentLoop::new(10);
    let state = agent.transition(ReactAction::Act {
        tool_name: "browser".into(),
        params: r#"{"url":"https://example.com"}"#.into(),
    });
    assert_eq!(state, ReactState::Acting);
}

#[test]
fn finish_transitions_to_done() {
    let mut agent = AgentLoop::new(10);
    agent.transition(ReactAction::Think);
    let state = agent.transition(ReactAction::Finish);
    assert_eq!(state, ReactState::Done);
}

#[test]
fn exceeding_max_turns_forces_done() {
    let mut agent = AgentLoop::new(2);
    agent.transition(ReactAction::Think);
    agent.transition(ReactAction::Observe);
    let state = agent.transition(ReactAction::Think);
    assert_eq!(
        state,
        ReactState::Done,
        "should be Done after exceeding max_turns"
    );
}

#[test]
fn loop_detection_triggers_after_repeated_calls() {
    let mut agent = AgentLoop::new(100);
    for _ in 0..3 {
        agent.transition(ReactAction::Act {
            tool_name: "search".into(),
            params: r#"{"q":"test"}"#.into(),
        });
    }
    assert!(agent.is_looping("search", r#"{"q":"test"}"#));
}

#[test]
fn idle_detection_after_noop_streak() {
    let mut agent = AgentLoop::new(100);
    assert!(!agent.is_idle());
    for _ in 0..3 {
        agent.transition(ReactAction::NoOp);
    }
    assert!(agent.is_idle());
}
