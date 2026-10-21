# Linux graphics renderer

GitComet defaults to **Automatic** on Linux, using Vulkan when available and
otherwise trying OpenGL. Choose **GitComet Settings → General → Graphics →
Graphics renderer** to select **Automatic**, **Vulkan**, or **OpenGL**.
Choosing Vulkan or OpenGL requires that backend; it does not fall back to the other.

Changes are saved before GitComet offers **Restart now** or **Later**. Restart
uses the normal unsaved-file, terminal, Git-operation, and extension guards.
Canceling a guard keeps the new preference saved and cancels the restart.
Focused Git diff/mergetool invocations apply changes on their next launch so
Git's waiting command is not ended by a settings restart.

The settings show the active backend separately from the saved preference.
Changing the preference does not switch the running renderer. Linux and Windows
renderer preferences are stored separately in the session file.

For diagnostics, `WGPU_BACKEND=vulkan` or `WGPU_BACKEND=gl` overrides the saved
preference. An override disables the settings choices without changing the
saved preference. Comma-separated overrides retain the existing Vulkan-first
selection order. Unset `WGPU_BACKEND` to use the saved preference again.

If a saved Vulkan or OpenGL choice prevents startup, close GitComet and set
`"linux_renderer": "auto"` in your `session.json`, or remove that key.
Preserve the other session data. GitComet normally uses
`$XDG_STATE_HOME/gitcomet/session.json`, falling back to
`~/.local/state/gitcomet/session.json`. If `GITCOMET_SESSION_FILE` is set, edit
that file instead. Launch without `WGPU_BACKEND` to use Automatic again.

OpenGL can fail even when the driver supports it. On Linux, wgpu's
[EGL configuration selection](https://github.com/gfx-rs/wgpu/blob/v30.0.1/wgpu-hal/src/gles/egl.rs#L99-L163)
requires a native-renderable window configuration. Some NVIDIA EGL
configurations do not provide one, leaving no compatible surface formats and
producing `No GPU adapter found that can configure the display surface`.
Use **Automatic** or **Vulkan**; if the app cannot open, reset the saved choice
as described above.
