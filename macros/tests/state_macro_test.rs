//! Tests for #[derive(State)] macro

use composable_rust_macros::State;
use composable_rust_core::stream::Version;

#[derive(State, Clone, Debug)]
struct TodoState {
    pub id: Option<String>,
    pub title: String,
    pub completed: bool,
    #[version]
    pub version: Option<Version>,
}

#[derive(State, Clone, Debug)]
struct SimpleState {
    pub count: i32,
}

#[test]
fn test_version_accessor() {
    let state = TodoState {
        id: Some("todo-1".to_string()),
        title: "Test".to_string(),
        completed: false,
        version: Some(Version::new(5)),
    };

    assert_eq!(state.version(), Some(Version::new(5)));
}

#[test]
fn test_set_version() {
    let mut state = TodoState {
        id: Some("todo-1".to_string()),
        title: "Test".to_string(),
        completed: false,
        version: None,
    };

    assert_eq!(state.version(), None);

    state.set_version(Version::new(10));
    assert_eq!(state.version(), Some(Version::new(10)));
}

#[test]
fn test_version_none() {
    let state = TodoState {
        id: None,
        title: String::new(),
        completed: false,
        version: None,
    };

    assert_eq!(state.version(), None);
}

#[test]
fn test_state_without_version() {
    // SimpleState doesn't have #[version], so it should compile
    // but not have version() and set_version() methods
    let _state = SimpleState { count: 0 };

    // This test just verifies compilation succeeds
    assert!(true);
}
