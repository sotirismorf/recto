# Flatpak packaging for Recto

## Prerequisites

```bash
flatpak remote-add --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo

flatpak install flathub \
    org.gnome.Platform//50 \
    org.gnome.Sdk//50 \
    org.freedesktop.Sdk.Extension.rust-stable//25.08 \
    org.freedesktop.Sdk.Extension.llvm20//25.08
```

## Build and install

```bash
flatpak-builder --user --install --force-clean build-dir flatpak/io.github.sotirismorf.Recto.json
```

## Run

```bash
flatpak run io.github.sotirismorf.Recto
```

## Open a project file

```bash
flatpak run io.github.sotirismorf.Recto /path/to/project.recto
```

## Regenerate crate sources

Run this whenever `Cargo.lock` changes (installed via pip):

```bash
flatpak-cargo-generator Cargo.lock -o flatpak/cargo-sources.json
```

