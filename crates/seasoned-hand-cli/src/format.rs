//! TTY-color helpers + table formatters.
//!
//! Status badge colors:
//! - running   → blue
//! - completed → green
//! - failed    → red
//! - paused    → yellow
//! - cancelled → bright_black (gray)
//! - drafted / briefed / confirmed → default (white)
//!
//! [`colored`] auto-detects TTY but `main.rs` also calls
//! `colored::control::set_override(...)` based on `--no-color` /
//! `--json` so the JSON path never embeds ANSI codes.

use colored::{ColoredString, Colorize};
use seasoned_hand_core::project::{Project, Task, TaskStatus};

pub fn task_status_badge(status: TaskStatus) -> ColoredString {
    let s = status.as_db_str();
    match status {
        TaskStatus::Running => s.blue().bold(),
        TaskStatus::Completed => s.green().bold(),
        TaskStatus::Failed => s.red().bold(),
        TaskStatus::Paused => s.yellow().bold(),
        TaskStatus::Cancelled => s.bright_black().bold(),
        TaskStatus::Drafted | TaskStatus::Briefed | TaskStatus::Confirmed => s.normal(),
    }
}

pub fn project_status_badge(status: seasoned_hand_core::project::ProjectStatus) -> ColoredString {
    use seasoned_hand_core::project::ProjectStatus;
    match status {
        ProjectStatus::Active => "active".green(),
        ProjectStatus::Archived => "archived".bright_black(),
    }
}

pub fn print_projects(projects: &[Project]) {
    if projects.is_empty() {
        println!("(no projects)");
        return;
    }
    println!(
        "{:<38}  {:<12}  {}",
        "ID".bold(),
        "STATUS".bold(),
        "TITLE".bold()
    );
    for p in projects {
        println!(
            "{:<38}  {:<12}  {}",
            p.id,
            project_status_badge(p.status),
            p.title
        );
    }
}

pub fn print_project(project: &Project) {
    println!("ID:          {}", project.id);
    println!("Title:       {}", project.title);
    println!("Status:      {}", project_status_badge(project.status));
    if let Some(desc) = &project.description {
        println!("Description: {desc}");
    }
    if let Some(tenant) = &project.tenant_id {
        println!("Tenant:      {tenant}");
    }
    println!("Created at:  {}", project.created_at);
}

pub fn print_tasks(tasks: &[Task]) {
    if tasks.is_empty() {
        println!("(no tasks)");
        return;
    }
    println!(
        "{:<38}  {:<10}  {}",
        "ID".bold(),
        "STATUS".bold(),
        "TITLE".bold()
    );
    for t in tasks {
        println!(
            "{:<38}  {:<10}  {}",
            t.id,
            task_status_badge(t.status),
            t.title
        );
    }
}

pub fn print_task(task: &Task) {
    println!("ID:         {}", task.id);
    println!("Project:    {}", task.project_id);
    println!("Title:      {}", task.title);
    println!("Status:     {}", task_status_badge(task.status));
    if let Some(reason) = &task.failure_reason {
        println!("Failure:    {reason}");
    }
    if let Some(due) = task.expected_due_at {
        println!("Due at:     {due}");
    }
    println!("Created at: {}", task.created_at);
}
