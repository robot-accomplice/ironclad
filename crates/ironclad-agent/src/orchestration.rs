use chrono::{DateTime, Utc};
use ironclad_core::{IroncladError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};

/// Orchestration pattern for multi-agent coordination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrchestrationPattern {
    Sequential,
    Parallel,
    FanOutFanIn,
    Handoff,
}

impl std::fmt::Display for OrchestrationPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrchestrationPattern::Sequential => write!(f, "sequential"),
            OrchestrationPattern::Parallel => write!(f, "parallel"),
            OrchestrationPattern::FanOutFanIn => write!(f, "fan-out/fan-in"),
            OrchestrationPattern::Handoff => write!(f, "handoff"),
        }
    }
}

/// A subtask assigned to a specialist agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subtask {
    pub id: String,
    pub description: String,
    pub required_capabilities: Vec<String>,
    pub assigned_agent: Option<String>,
    pub status: SubtaskStatus,
    pub result: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Status of a subtask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubtaskStatus {
    Pending,
    Assigned,
    Running,
    Completed,
    Failed,
}

/// A workflow composed of subtasks with an orchestration pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub pattern: OrchestrationPattern,
    pub subtasks: Vec<Subtask>,
    pub status: WorkflowStatus,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Status of a workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowStatus {
    Created,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Coordinates workflows and agent assignment.
pub struct Orchestrator {
    workflows: HashMap<String, Workflow>,
    workflow_counter: u64,
}

impl Orchestrator {
    pub fn new() -> Self {
        Self {
            workflows: HashMap::new(),
            workflow_counter: 0,
        }
    }

    /// Create a new workflow from subtask descriptions.
    pub fn create_workflow(
        &mut self,
        name: &str,
        pattern: OrchestrationPattern,
        subtasks: Vec<(String, Vec<String>)>,
    ) -> String {
        self.workflow_counter += 1;
        let workflow_id = format!("wf_{}", self.workflow_counter);

        let tasks: Vec<Subtask> = subtasks
            .into_iter()
            .enumerate()
            .map(|(i, (desc, caps))| Subtask {
                id: format!("{}_task_{}", workflow_id, i),
                description: desc,
                required_capabilities: caps,
                assigned_agent: None,
                status: SubtaskStatus::Pending,
                result: None,
                created_at: Utc::now(),
                completed_at: None,
            })
            .collect();

        let workflow = Workflow {
            id: workflow_id.clone(),
            name: name.to_string(),
            pattern,
            subtasks: tasks,
            status: WorkflowStatus::Created,
            created_at: Utc::now(),
            completed_at: None,
        };

        info!(id = %workflow_id, name, pattern = %pattern, tasks = workflow.subtasks.len(), "created workflow");
        self.workflows.insert(workflow_id.clone(), workflow);
        workflow_id
    }

    /// Assign an agent to a subtask.
    pub fn assign_agent(&mut self, workflow_id: &str, task_id: &str, agent_id: &str) -> Result<()> {
        let workflow = self.workflows.get_mut(workflow_id).ok_or_else(|| {
            IroncladError::Config(format!("workflow '{}' not found", workflow_id))
        })?;

        let task = workflow
            .subtasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| IroncladError::Config(format!("task '{}' not found", task_id)))?;

        task.assigned_agent = Some(agent_id.to_string());
        task.status = SubtaskStatus::Assigned;
        debug!(
            workflow = workflow_id,
            task = task_id,
            agent = agent_id,
            "agent assigned"
        );
        Ok(())
    }

    /// Match subtasks to available agents by capability overlap.
    pub fn match_capabilities(
        &self,
        workflow_id: &str,
        available_agents: &[(String, Vec<String>)],
    ) -> Result<Vec<(String, String)>> {
        let workflow = self.workflows.get(workflow_id).ok_or_else(|| {
            IroncladError::Config(format!("workflow '{}' not found", workflow_id))
        })?;

        let mut assignments = Vec::new();

        for task in &workflow.subtasks {
            if task.status != SubtaskStatus::Pending {
                continue;
            }

            let best_agent = available_agents.iter().max_by_key(|(_, caps)| {
                task.required_capabilities
                    .iter()
                    .filter(|rc| caps.contains(rc))
                    .count()
            });

            if let Some((agent_id, caps)) = best_agent {
                let overlap = task
                    .required_capabilities
                    .iter()
                    .filter(|rc| caps.contains(rc))
                    .count();
                if overlap > 0 {
                    assignments.push((task.id.clone(), agent_id.clone()));
                }
            }
        }

        Ok(assignments)
    }

    /// Start a subtask.
    pub fn start_task(&mut self, workflow_id: &str, task_id: &str) -> Result<()> {
        let workflow = self.workflows.get_mut(workflow_id).ok_or_else(|| {
            IroncladError::Config(format!("workflow '{}' not found", workflow_id))
        })?;

        let task = workflow
            .subtasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| IroncladError::Config(format!("task '{}' not found", task_id)))?;

        task.status = SubtaskStatus::Running;
        workflow.status = WorkflowStatus::Running;
        Ok(())
    }

    /// Complete a subtask with a result.
    pub fn complete_task(
        &mut self,
        workflow_id: &str,
        task_id: &str,
        result: String,
    ) -> Result<()> {
        let workflow = self.workflows.get_mut(workflow_id).ok_or_else(|| {
            IroncladError::Config(format!("workflow '{}' not found", workflow_id))
        })?;

        let task = workflow
            .subtasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| IroncladError::Config(format!("task '{}' not found", task_id)))?;

        task.status = SubtaskStatus::Completed;
        task.result = Some(result);
        task.completed_at = Some(Utc::now());

        if workflow
            .subtasks
            .iter()
            .all(|t| t.status == SubtaskStatus::Completed)
        {
            workflow.status = WorkflowStatus::Completed;
            workflow.completed_at = Some(Utc::now());
            info!(id = %workflow_id, "workflow completed");
        }

        Ok(())
    }

    /// Fail a subtask.
    pub fn fail_task(&mut self, workflow_id: &str, task_id: &str, error: &str) -> Result<()> {
        let workflow = self.workflows.get_mut(workflow_id).ok_or_else(|| {
            IroncladError::Config(format!("workflow '{}' not found", workflow_id))
        })?;

        let task = workflow
            .subtasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| IroncladError::Config(format!("task '{}' not found", task_id)))?;

        task.status = SubtaskStatus::Failed;
        task.result = Some(format!("ERROR: {}", error));
        task.completed_at = Some(Utc::now());

        workflow.status = WorkflowStatus::Failed;
        warn!(workflow = workflow_id, task = task_id, error, "task failed");
        Ok(())
    }

    /// Get a workflow by ID.
    pub fn get_workflow(&self, id: &str) -> Option<&Workflow> {
        self.workflows.get(id)
    }

    /// Get the next actionable tasks for a workflow based on its pattern.
    pub fn next_tasks(&self, workflow_id: &str) -> Result<Vec<&Subtask>> {
        let workflow = self.workflows.get(workflow_id).ok_or_else(|| {
            IroncladError::Config(format!("workflow '{}' not found", workflow_id))
        })?;

        match workflow.pattern {
            OrchestrationPattern::Sequential => Ok(workflow
                .subtasks
                .iter()
                .find(|t| t.status == SubtaskStatus::Pending || t.status == SubtaskStatus::Assigned)
                .into_iter()
                .collect()),
            OrchestrationPattern::Parallel | OrchestrationPattern::FanOutFanIn => Ok(workflow
                .subtasks
                .iter()
                .filter(|t| {
                    t.status == SubtaskStatus::Pending || t.status == SubtaskStatus::Assigned
                })
                .collect()),
            OrchestrationPattern::Handoff => {
                let last_completed = workflow
                    .subtasks
                    .iter()
                    .rposition(|t| t.status == SubtaskStatus::Completed);
                let next_idx = last_completed.map(|i| i + 1).unwrap_or(0);
                Ok(workflow.subtasks.get(next_idx).into_iter().collect())
            }
        }
    }

    pub fn workflow_count(&self) -> usize {
        self.workflows.len()
    }
}

impl Default for Orchestrator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_tasks() -> Vec<(String, Vec<String>)> {
        vec![
            ("Research the topic".into(), vec!["research".into()]),
            ("Write the summary".into(), vec!["summarization".into()]),
            ("Review the output".into(), vec!["review".into()]),
        ]
    }

    #[test]
    fn create_workflow() {
        let mut orch = Orchestrator::new();
        let id = orch.create_workflow(
            "Test Flow",
            OrchestrationPattern::Sequential,
            simple_tasks(),
        );
        assert!(id.starts_with("wf_"));
        let wf = orch.get_workflow(&id).unwrap();
        assert_eq!(wf.subtasks.len(), 3);
        assert_eq!(wf.status, WorkflowStatus::Created);
    }

    #[test]
    fn assign_and_start() {
        let mut orch = Orchestrator::new();
        let wf_id = orch.create_workflow("Test", OrchestrationPattern::Sequential, simple_tasks());
        let task_id = orch.get_workflow(&wf_id).unwrap().subtasks[0].id.clone();

        orch.assign_agent(&wf_id, &task_id, "agent-research")
            .unwrap();
        let task = &orch.get_workflow(&wf_id).unwrap().subtasks[0];
        assert_eq!(task.status, SubtaskStatus::Assigned);
        assert_eq!(task.assigned_agent.as_deref(), Some("agent-research"));

        orch.start_task(&wf_id, &task_id).unwrap();
        assert_eq!(
            orch.get_workflow(&wf_id).unwrap().subtasks[0].status,
            SubtaskStatus::Running
        );
    }

    #[test]
    fn complete_workflow() {
        let mut orch = Orchestrator::new();
        let wf_id = orch.create_workflow("Test", OrchestrationPattern::Parallel, simple_tasks());
        let task_ids: Vec<String> = orch
            .get_workflow(&wf_id)
            .unwrap()
            .subtasks
            .iter()
            .map(|t| t.id.clone())
            .collect();

        for tid in &task_ids {
            orch.complete_task(&wf_id, tid, "done".into()).unwrap();
        }

        let wf = orch.get_workflow(&wf_id).unwrap();
        assert_eq!(wf.status, WorkflowStatus::Completed);
        assert!(wf.completed_at.is_some());
    }

    #[test]
    fn fail_task_fails_workflow() {
        let mut orch = Orchestrator::new();
        let wf_id = orch.create_workflow("Test", OrchestrationPattern::Sequential, simple_tasks());
        let task_id = orch.get_workflow(&wf_id).unwrap().subtasks[0].id.clone();

        orch.fail_task(&wf_id, &task_id, "something broke").unwrap();
        assert_eq!(
            orch.get_workflow(&wf_id).unwrap().status,
            WorkflowStatus::Failed
        );
    }

    #[test]
    fn sequential_next_tasks() {
        let mut orch = Orchestrator::new();
        let wf_id = orch.create_workflow("Seq", OrchestrationPattern::Sequential, simple_tasks());

        let next = orch.next_tasks(&wf_id).unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].description, "Research the topic");
    }

    #[test]
    fn parallel_next_tasks() {
        let mut orch = Orchestrator::new();
        let wf_id = orch.create_workflow("Par", OrchestrationPattern::Parallel, simple_tasks());

        let next = orch.next_tasks(&wf_id).unwrap();
        assert_eq!(next.len(), 3);
    }

    #[test]
    fn capability_matching() {
        let mut orch = Orchestrator::new();
        let wf_id = orch.create_workflow("Match", OrchestrationPattern::Parallel, simple_tasks());

        let agents = vec![
            (
                "researcher".into(),
                vec!["research".into(), "analysis".into()],
            ),
            (
                "writer".into(),
                vec!["summarization".into(), "writing".into()],
            ),
        ];

        let matches = orch.match_capabilities(&wf_id, &agents).unwrap();
        assert!(!matches.is_empty());
    }

    #[test]
    fn pattern_display() {
        assert_eq!(
            format!("{}", OrchestrationPattern::Sequential),
            "sequential"
        );
        assert_eq!(format!("{}", OrchestrationPattern::Parallel), "parallel");
        assert_eq!(
            format!("{}", OrchestrationPattern::FanOutFanIn),
            "fan-out/fan-in"
        );
        assert_eq!(format!("{}", OrchestrationPattern::Handoff), "handoff");
    }

    #[test]
    fn pattern_serde() {
        for pattern in [
            OrchestrationPattern::Sequential,
            OrchestrationPattern::Parallel,
            OrchestrationPattern::FanOutFanIn,
            OrchestrationPattern::Handoff,
        ] {
            let json = serde_json::to_string(&pattern).unwrap();
            let back: OrchestrationPattern = serde_json::from_str(&json).unwrap();
            assert_eq!(pattern, back);
        }
    }

    #[test]
    fn workflow_not_found() {
        let orch = Orchestrator::new();
        assert!(orch.get_workflow("nope").is_none());
        assert!(orch.next_tasks("nope").is_err());
    }
}
