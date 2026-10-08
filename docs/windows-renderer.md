# Windows graphics renderer

GitComet defaults to **Automatic** on Windows. It checks hardware DirectX 12
support before creating the DX12 renderer. If the check or initialization fails,
it saves **DirectX 11 — compatibility** in the session and starts with DX11.
Future launches read that preference and skip DX12 detection and initialization.

Change **GitComet Settings → General → Graphics → Graphics renderer** to
**Automatic** to try DX12 again, including after updating a graphics driver.
GitComet does not retry automatically after driver or application updates.

Manual preference changes are saved before GitComet offers **Restart now** or
**Later**. Restart uses the normal unsaved-file, terminal, Git-operation, and
extension guards. Canceling a guard leaves the new preference saved but cancels
the restart. Focused Git diff/mergetool invocations apply changes on their next
launch so Git's waiting command is not ended by a settings restart.

Recoverable device loss does not change the preference. A confirmed DX12
rendering failure or exhausted recovery saves DX11 for the next launch before
normal fatal handling. The running session does not switch graphics APIs.
If saving fails, the log and settings explain that the next launch may retry DX12.

For diagnostics, `GPUI_WINDOWS_RENDERER=dx11` or
`GPUI_WINDOWS_RENDERER=wgpu-dx12` strictly overrides the saved preference.
Forced DX12 does not fall back and overrides do not write the preference.

Production Windows builds include both renderer paths without the experimental
GPU benchmark instrumentation. DX12 retains a DX11 device for Windows text and
support services. Automatic chooses a usable renderer; it does not run a
performance comparison during startup.
