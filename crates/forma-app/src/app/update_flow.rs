use super::*;
use crate::updater::State;

impl Studio {
    pub(super) fn execute_update(
        &mut self,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            Command::CheckForUpdates => {
                self.updates.open = true;
                self.updates.check(true);
            }
            Command::ToggleUpdates => self.updates.open = !self.updates.open,
            Command::DownloadUpdate => self.updates.download(),
            Command::SkipUpdate => self.updates.skip(),
            Command::ToggleUpdateChecks => {
                self.updates.preferences.automatic_checks =
                    !self.updates.preferences.automatic_checks;
                self.updates.save_preferences();
            }
            Command::ToggleUpdateDownloads => {
                self.updates.preferences.automatic_downloads =
                    !self.updates.preferences.automatic_downloads;
                self.updates.save_preferences();
            }
            Command::RestartUpdate => self.request_update_restart(window, cx),
            _ => unreachable!(),
        }
        if self.updates.open {
            self.navigation = None;
            self.focus.focus(window);
        }
        cx.notify();
    }

    fn update_busy(&self) -> bool {
        self.saving
            || self.loading
            || self.exporting
            || self.exporting_obj
            || self.exporting_stl
            || self.texture_loading
            || self.preview_loading
            || self.shader_compiling
    }

    fn request_update_restart(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.updates.state, State::Ready(_)) || self.prompt_open {
            return;
        }
        if self.update_busy() {
            self.status = "Finish the current save, import or export before restarting".into();
            return;
        }
        if self.transform_drag.is_some() || self.shader_editor.is_some() {
            self.status = "Finish the current edit before restarting".into();
            return;
        }
        if self.active_field.is_some() {
            self.commit_field(cx);
            if self.active_field.is_some() {
                return;
            }
        }
        if !self.dirty {
            self.begin_update_install(cx);
            return;
        }
        self.prompt_open = true;
        let original = (self.project_epoch, self.generation);
        let answer = window.prompt(
            PromptLevel::Warning,
            "Restart Forma to update?",
            Some("Save this project before restarting to keep your work."),
            &["Cancel", "Save and restart", "Discard and restart"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            let choice = answer.await.ok();
            let _ = this.update(cx, |s, cx| {
                s.prompt_open = false;
                if original != (s.project_epoch, s.generation) || s.update_busy() {
                    s.status =
                        "Project changed while confirming. Restart the update when ready.".into();
                    cx.notify();
                    return;
                }
                match choice {
                    Some(1) => s.save_for_update(cx),
                    Some(2) => s.begin_update_install(cx),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn save_for_update(&mut self, cx: &mut Context<Self>) {
        self.updates.restart_pending = true;
        if let Some(path) = self.path.clone() {
            self.save_to(path, cx);
            return;
        }
        let directory = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let receiver =
            cx.prompt_for_new_path(&directory, Some(&format!("{}.forma", self.project_name)));
        let original = (self.project_epoch, self.generation);
        cx.spawn(async move |this, cx| {
            let result = receiver.await;
            let _ = this.update(cx, |s, cx| {
                match result {
                    Ok(Ok(Some(mut path)))
                        if original == (s.project_epoch, s.generation) && !s.update_busy() =>
                    {
                        if path.extension().is_none() {
                            path.set_extension("forma");
                        }
                        s.save_to(path, cx);
                    }
                    _ => {
                        s.updates.restart_pending = false;
                        s.status = "Update restart cancelled".into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn finish_update_save(
        &mut self,
        saved: bool,
        original: (u64, u64),
        cx: &mut Context<Self>,
    ) {
        if forma_update::may_restart(
            saved,
            self.dirty,
            self.update_busy(),
            original,
            (self.project_epoch, self.generation),
        ) {
            self.begin_update_install(cx);
        } else {
            self.updates.restart_pending = false;
            if saved {
                self.status =
                    "Project changed or another operation started. Update restart cancelled."
                        .into();
            }
        }
    }

    fn begin_update_install(&mut self, cx: &mut Context<Self>) {
        let mut args = vec!["--renderer".into(), self.renderer_backend.to_string()];
        if let Some(path) = &self.path {
            let Some(path) = path.to_str() else {
                self.updates.restart_pending = false;
                self.status = "Save to a Unicode project path before restarting".into();
                return;
            };
            args.extend(["--open".into(), path.into()]);
        }
        self.updates.restart_pending = true;
        self.updates.install(args);
        cx.notify();
    }
}
