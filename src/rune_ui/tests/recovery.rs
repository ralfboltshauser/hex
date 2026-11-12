//! Recovery controls use the same headless event/accessibility surface.
use super::*;

#[test]
fn recovery_shows_saved_text_and_names_its_controls() {
    if let Some(folder) = std::env::var_os("HEX_GOLDEN_RENDERS") {
        let folder = std::path::PathBuf::from(folder);
        std::fs::create_dir_all(&folder).unwrap();
        for dark in [true, false] {
            let (width, height, pixels) =
                render("history-recovery", dark, PANE_SIZE).expect("GPU rendering");
            let theme = if dark { "dark" } else { "light" };
            rune::render::write_png(
                &folder.join(format!("history-recovery-{theme}.png")),
                width,
                height,
                &pixels,
            )
            .unwrap();
        }
    }
    let mut rt = runtime("history-recovery", true, PANE_SIZE);
    assert!(rt.find("recovery-detail").is_some());
    tap(&mut rt, "recovery-copy");
    assert_eq!(
        rt.copy_request.as_deref(),
        Some("The saved transcript remains available after an interrupted delivery.")
    );
    tap(&mut rt, "recovery-delete");
    assert!(rt.find("recovery-delete").is_some());
    tap(&mut rt, "history-recovery-filter");
    assert!(rt.find("recovery-detail").is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn recovery_dark_and_light_goldens() {
    check_golden("history-recovery", true, PANE_SIZE);
    check_golden("history-recovery", false, PANE_SIZE);
}
