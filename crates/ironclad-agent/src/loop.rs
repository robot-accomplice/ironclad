use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReactState {
    Thinking,
    Acting,
    Observing,
    Persisting,
    Idle,
    Done,
}

#[derive(Debug, Clone)]
pub enum ReactAction {
    Think,
    Act { tool_name: String, params: String },
    Observe,
    Persist,
    NoOp,
    Finish,
}

const IDLE_THRESHOLD: usize = 3;
const LOOP_DETECTION_WINDOW: usize = 3;

pub struct AgentLoop {
    pub state: ReactState,
    pub turn_count: usize,
    pub max_turns: usize,
    idle_count: usize,
    recent_calls: VecDeque<(String, String)>,
}

impl AgentLoop {
    pub fn new(max_turns: usize) -> Self {
        Self {
            state: ReactState::Idle,
            turn_count: 0,
            max_turns,
            idle_count: 0,
            recent_calls: VecDeque::with_capacity(LOOP_DETECTION_WINDOW + 1),
        }
    }

    pub fn transition(&mut self, action: ReactAction) -> ReactState {
        self.turn_count += 1;

        if self.turn_count > self.max_turns {
            self.state = ReactState::Done;
            return self.state;
        }

        match action {
            ReactAction::Think => {
                self.idle_count = 0;
                self.state = ReactState::Thinking;
            }
            ReactAction::Act { tool_name, params } => {
                self.idle_count = 0;
                self.recent_calls
                    .push_back((tool_name.clone(), params.clone()));
                if self.recent_calls.len() > LOOP_DETECTION_WINDOW {
                    self.recent_calls.pop_front();
                }
                if self.is_looping(&tool_name, &params) {
                    tracing::warn!(tool = %tool_name, "agent loop detected, forcing Done");
                    self.state = ReactState::Done;
                } else {
                    self.state = ReactState::Acting;
                }
            }
            ReactAction::Observe => {
                self.idle_count = 0;
                self.state = ReactState::Observing;
            }
            ReactAction::Persist => {
                self.idle_count = 0;
                self.state = ReactState::Persisting;
            }
            ReactAction::NoOp => {
                self.idle_count += 1;
                if self.idle_count >= IDLE_THRESHOLD {
                    self.state = ReactState::Idle;
                }
            }
            ReactAction::Finish => {
                self.state = ReactState::Done;
            }
        }

        self.state
    }

    pub fn is_idle(&self) -> bool {
        self.idle_count >= IDLE_THRESHOLD
    }

    /// Returns true if the same tool+params combination has appeared
    /// `LOOP_DETECTION_WINDOW` consecutive times.
    pub fn is_looping(&self, tool_name: &str, params: &str) -> bool {
        if self.recent_calls.len() < LOOP_DETECTION_WINDOW {
            return false;
        }

        self.recent_calls
            .iter()
            .all(|(t, p)| t == tool_name && p == params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_transitions() {
        let mut agent = AgentLoop::new(100);
        assert_eq!(agent.state, ReactState::Idle);

        let s = agent.transition(ReactAction::Think);
        assert_eq!(s, ReactState::Thinking);

        let s = agent.transition(ReactAction::Act {
            tool_name: "echo".into(),
            params: "{}".into(),
        });
        assert_eq!(s, ReactState::Acting);

        let s = agent.transition(ReactAction::Observe);
        assert_eq!(s, ReactState::Observing);

        let s = agent.transition(ReactAction::Persist);
        assert_eq!(s, ReactState::Persisting);

        let s = agent.transition(ReactAction::Finish);
        assert_eq!(s, ReactState::Done);
    }

    #[test]
    fn idle_detection() {
        let mut agent = AgentLoop::new(100);

        assert!(!agent.is_idle());
        agent.transition(ReactAction::NoOp);
        assert!(!agent.is_idle());
        agent.transition(ReactAction::NoOp);
        assert!(!agent.is_idle());
        agent.transition(ReactAction::NoOp);
        assert!(agent.is_idle());
        assert_eq!(agent.state, ReactState::Idle);

        agent.transition(ReactAction::Think);
        assert!(!agent.is_idle());
    }

    #[test]
    fn loop_detection() {
        let mut agent = AgentLoop::new(100);

        for _ in 0..3 {
            agent.transition(ReactAction::Act {
                tool_name: "echo".into(),
                params: r#"{"msg":"hi"}"#.into(),
            });
        }

        assert!(agent.is_looping("echo", r#"{"msg":"hi"}"#));
        assert!(!agent.is_looping("echo", r#"{"msg":"bye"}"#));
        assert!(!agent.is_looping("other", r#"{"msg":"hi"}"#));

        agent.transition(ReactAction::Act {
            tool_name: "read".into(),
            params: "{}".into(),
        });
        assert!(!agent.is_looping("echo", r#"{"msg":"hi"}"#));
    }

    #[test]
    fn max_turns_forces_done() {
        let mut agent = AgentLoop::new(2);

        agent.transition(ReactAction::Think);
        agent.transition(ReactAction::Think);
        let s = agent.transition(ReactAction::Think);
        assert_eq!(s, ReactState::Done);
    }
}
