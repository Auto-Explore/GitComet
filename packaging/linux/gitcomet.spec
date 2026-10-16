# Binary RPM built by scripts/package-linux.sh from a staged payload.
# gc_version, gc_release and gc_payload are passed with --define.
%global debug_package %{nil}
%global __os_install_post %{nil}

Name:           gitcomet
Version:        %{gc_version}
Release:        %{gc_release}
Summary:        Fast, resource-efficient Git GUI written in Rust
License:        AGPL-3.0-only
URL:            https://gitcomet.dev/

# Linked libraries come from AutoReq. Runtime-loaded ones are declared here and
# checked by scripts/check-linux-runtime-deps.sh.
Requires:       git
Requires:       hicolor-icon-theme
# The renderer tries Vulkan, then EGL/GL.
Requires:       (vulkan-loader or libglvnd-egl)
Recommends:     vulkan-loader
# Native Wayland; without these the app runs through X11.
Recommends:     libwayland-client
Recommends:     libwayland-egl
Suggests:       git-lfs
Suggests:       gnupg2
Suggests:       openssh-clients
Suggests:       xdg-utils
Suggests:       xdg-desktop-portal
Suggests:       google-noto-sans-cjk-fonts
Suggests:       google-noto-color-emoji-fonts

%description
Fast, resource-efficient Git GUI written in Rust.

%install
cp -a %{gc_payload}/. %{buildroot}/

%files
%{_bindir}/gitcomet
%{_datadir}/applications/gitcomet.desktop
%{_datadir}/icons/hicolor/*/apps/gitcomet.png
%license %{_datadir}/licenses/gitcomet/LICENSE-AGPL-3.0
%license %{_datadir}/licenses/gitcomet/NOTICE
%doc %{_datadir}/doc/gitcomet/README.md
