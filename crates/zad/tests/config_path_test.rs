use std::path::PathBuf;

use zad::config::path::{project_dir_for, project_slug_for};

#[test]
fn slug_uses_dash_for_slash_preserving_leading_separator() {
    let p = PathBuf::from("/Users/niclas/Source/personal/oss/zad");
    let slug = project_slug_for(&p).unwrap();
    assert_eq!(slug, "-Users-niclas-Source-personal-oss-zad");
}

#[test]
fn slug_handles_root_alone() {
    let p = PathBuf::from("/");
    let slug = project_slug_for(&p).unwrap();
    assert_eq!(slug, "-");
}

#[test]
fn slug_collapses_windows_separators_and_drive_colon() {
    let p = PathBuf::from(r"C:\Users\niclas\repo");
    let slug = project_slug_for(&p).unwrap();
    assert_eq!(slug, "C--Users-niclas-repo");
}

#[test]
fn project_dir_composes_under_zad_home() {
    let tmp = tempfile::tempdir().unwrap();
    // SAFETY: std::env::set_var is unsafe only because of rare race
    // conditions with concurrent threads reading the env. In a single-
    // threaded #[test] body this is fine.
    unsafe {
        std::env::set_var("ZAD_HOME_OVERRIDE", tmp.path());
    }
    let dir = project_dir_for("-sample").unwrap();
    assert_eq!(
        dir,
        tmp.path().join(".zad").join("projects").join("-sample")
    );
    unsafe {
        std::env::remove_var("ZAD_HOME_OVERRIDE");
    }
}
