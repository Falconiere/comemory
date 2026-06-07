//! Integration tests for `comemory completions <shell>`.

use assert_cmd::Command;

fn run_completions(shell: &str) -> assert_cmd::assert::Assert {
    Command::cargo_bin("comemory")
        .expect("cargo_bin comemory")
        .args(["completions", shell])
        .assert()
}

#[test]
fn fish_emits_completion_script() {
    let out = run_completions("fish")
        .success()
        .get_output()
        .stdout
        .clone();
    let body = String::from_utf8(out).expect("fish completions are utf-8");
    assert!(!body.trim().is_empty(), "fish completions stdout is empty");
    assert!(
        body.contains("comemory"),
        "fish completions missing binary name"
    );
}

#[test]
fn bash_emits_completion_script() {
    let out = run_completions("bash")
        .success()
        .get_output()
        .stdout
        .clone();
    let body = String::from_utf8(out).expect("bash completions are utf-8");
    assert!(!body.trim().is_empty(), "bash completions stdout is empty");
    assert!(
        body.contains("comemory"),
        "bash completions missing binary name"
    );
}

#[test]
fn zsh_emits_completion_script() {
    let out = run_completions("zsh").success().get_output().stdout.clone();
    let body = String::from_utf8(out).expect("zsh completions are utf-8");
    assert!(!body.trim().is_empty(), "zsh completions stdout is empty");
    assert!(
        body.contains("comemory"),
        "zsh completions missing binary name"
    );
}

#[test]
fn powershell_emits_completion_script() {
    let out = run_completions("powershell")
        .success()
        .get_output()
        .stdout
        .clone();
    let body = String::from_utf8(out).expect("powershell completions are utf-8");
    assert!(
        !body.trim().is_empty(),
        "powershell completions stdout is empty"
    );
    assert!(
        body.contains("comemory"),
        "powershell completions missing binary name"
    );
}

#[test]
fn elvish_emits_completion_script() {
    let out = run_completions("elvish")
        .success()
        .get_output()
        .stdout
        .clone();
    let body = String::from_utf8(out).expect("elvish completions are utf-8");
    assert!(
        !body.trim().is_empty(),
        "elvish completions stdout is empty"
    );
    assert!(
        body.contains("comemory"),
        "elvish completions missing binary name"
    );
}
