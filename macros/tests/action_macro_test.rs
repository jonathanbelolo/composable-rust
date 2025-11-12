//! Tests for #[derive(Action)] macro

use composable_rust_macros::Action;
use chrono::{DateTime, Utc};

#[derive(Action, Clone, Debug, PartialEq)]
enum TodoAction {
    #[command]
    CreateTodo {
        title: String,
    },

    #[command]
    ToggleTodo,

    #[command]
    UpdateTitle {
        new_title: String,
    },

    #[event]
    TodoCreated {
        id: String,
        title: String,
        timestamp: DateTime<Utc>,
    },

    #[event]
    TodoToggled {
        completed: bool,
        timestamp: DateTime<Utc>,
    },

    #[event]
    TitleUpdated {
        new_title: String,
        timestamp: DateTime<Utc>,
    },
}

#[test]
fn test_is_command() {
    let action = TodoAction::CreateTodo {
        title: "Test".to_string(),
    };
    assert!(action.is_command());
    assert!(!action.is_event());
}

#[test]
fn test_is_event() {
    let action = TodoAction::TodoCreated {
        id: "todo-1".to_string(),
        title: "Test".to_string(),
        timestamp: Utc::now(),
    };
    assert!(!action.is_command());
    assert!(action.is_event());
}

#[test]
fn test_event_type() {
    let action = TodoAction::TodoCreated {
        id: "todo-1".to_string(),
        title: "Test".to_string(),
        timestamp: Utc::now(),
    };
    assert_eq!(action.event_type(), "TodoCreated.v1");
}

#[test]
fn test_command_event_type() {
    let action = TodoAction::CreateTodo {
        title: "Test".to_string(),
    };
    // Commands don't have event types
    assert_eq!(action.event_type(), "unknown");
}

#[test]
fn test_toggle_command() {
    let action = TodoAction::ToggleTodo;
    assert!(action.is_command());
    assert!(!action.is_event());
}

#[test]
fn test_all_commands_identified() {
    let commands = vec![
        TodoAction::CreateTodo {
            title: "Test".to_string(),
        },
        TodoAction::ToggleTodo,
        TodoAction::UpdateTitle {
            new_title: "New".to_string(),
        },
    ];

    for cmd in commands {
        assert!(cmd.is_command(), "Expected command: {cmd:?}");
        assert!(!cmd.is_event(), "Should not be event: {cmd:?}");
    }
}

#[test]
fn test_all_events_identified() {
    let events = vec![
        TodoAction::TodoCreated {
            id: "1".to_string(),
            title: "Test".to_string(),
            timestamp: Utc::now(),
        },
        TodoAction::TodoToggled {
            completed: true,
            timestamp: Utc::now(),
        },
        TodoAction::TitleUpdated {
            new_title: "New".to_string(),
            timestamp: Utc::now(),
        },
    ];

    for event in events {
        assert!(!event.is_command(), "Should not be command: {event:?}");
        assert!(event.is_event(), "Expected event: {event:?}");
    }
}

#[test]
fn test_event_types_unique() {
    let events = vec![
        (
            TodoAction::TodoCreated {
                id: "1".to_string(),
                title: "Test".to_string(),
                timestamp: Utc::now(),
            },
            "TodoCreated.v1",
        ),
        (
            TodoAction::TodoToggled {
                completed: true,
                timestamp: Utc::now(),
            },
            "TodoToggled.v1",
        ),
        (
            TodoAction::TitleUpdated {
                new_title: "New".to_string(),
                timestamp: Utc::now(),
            },
            "TitleUpdated.v1",
        ),
    ];

    for (event, expected_type) in events {
        assert_eq!(event.event_type(), expected_type);
    }
}
