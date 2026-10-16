//! Native editor regression checks for update restart safety.
use super::*;
use crate::smoke::{capture_window, keys, prepare_capture, wait_for};
use anyhow::ensure;
use gpui::{AsyncApp, WindowHandle};

pub(crate) async fn check(
    window: WindowHandle<Studio>,
    output: &std::path::Path,
    cx: &mut AsyncApp,
) -> Result<()> {
    use crate::updater::State;
    let saved = window.update(cx, |s, w, cx| {
        let saved = (
            s.updates.state.clone(),
            s.path.clone(),
            s.project_name.clone(),
            s.dirty,
        );
        s.execute(Command::CheckForUpdates, w, cx);
        saved
    })?;
    let original = window.update(cx, |s, _, _| (s.project_epoch, s.generation))?;
    keys(window, &["cmd-n", "g", "delete"], cx).await?;
    window.update(cx, |s, w, cx| -> Result<()> {
        s.execute(Command::New, w, cx);
        ensure!(
            s.updates.open && s.navigation_blocked(),
            "update dialog did not block editor input"
        );
        ensure!(
            original == (s.project_epoch, s.generation),
            "update dialog let an edit through"
        );
        s.updates.receive(State::Ready("1.0.1".into()));
        s.exporting = true;
        s.execute(Command::RestartUpdate, w, cx);
        ensure!(
            !s.updates.restart_pending && matches!(s.updates.state, State::Ready(_)),
            "restart interrupted an export"
        );
        s.exporting = false;
        Ok(())
    })??;
    prepare_capture(window, cx).await?;
    window.update(cx, |s, w, _| {
        capture_window(s, w, &output.join("updates-ready.png"))
    })??;
    keys(window, &["escape"], cx).await?;

    // Smoke sessions have no updater worker. A successful save can reach the
    // install request, which then fails safely without replacing the test app.
    let path = output.join("update-saved.forma");
    window.update(cx, |s, w, cx| -> Result<()> {
        s.dirty = true;
        s.updates.restart_pending = true;
        s.save_to(path.clone(), cx);
        ensure!(
            !s.accepts_document_result(original.0, original.1),
            "late file dialog could change the project during restart"
        );
        s.execute(Command::New, w, cx);
        ensure!(
            original == (s.project_epoch, s.generation),
            "input changed the project during update save"
        );
        Ok(())
    })??;
    wait_for(window, cx, |s| !s.saving).await?;
    window.update(cx, |s, _, _| -> Result<()> {
        ensure!(
            !s.dirty && !s.updates.restart_pending && matches!(s.updates.state, State::Failed(_)),
            "saved update did not reach the guarded install request"
        );
        ensure!(
            Scene::load(&path).is_ok(),
            "update restart did not persist the project"
        );
        Ok(())
    })??;
    // Saving over a directory must fail and keep dirty work in the editor.
    window.update(cx, |s, _, cx| {
        s.dirty = true;
        s.updates.receive(State::Ready("1.0.1".into()));
        s.updates.restart_pending = true;
        s.save_to(output.to_path_buf(), cx);
    })?;
    wait_for(window, cx, |s| !s.saving).await?;
    window.update(cx, |s, _, cx| -> Result<()> {
        ensure!(
            s.dirty && !s.updates.restart_pending && matches!(s.updates.state, State::Ready(_)),
            "failed save attempted an update or lost dirty work"
        );
        s.updates.receive(saved.0);
        s.path = saved.1;
        s.project_name = saved.2;
        s.dirty = saved.3;
        s.status = "Update workflow checks passed".into();
        cx.notify();
        Ok(())
    })??;
    println!(
        "updates=modal_input_export_guard_save_before_install_failed_save_preserves_work PASS"
    );
    Ok(())
}
