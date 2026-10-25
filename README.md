# AppImgManager

A small tool for managing AppImage files on Linux.
Track an AppImage and it gets a proper home, a menu entry and an icon, so it behaves like any other installed app.

---

## Installation

### Install script

Installs the latest release binary for your architecture (`x86_64` or `aarch64`) and registers AppImgManager as an "Open with" option for AppImage files.

```sh
curl -fsSL https://raw.githubusercontent.com/Julius-IX/AppImgManager/main/scripts/install.sh | bash
```

Pin a specific version:

```sh
curl -fsSL https://raw.githubusercontent.com/Julius-IX/AppImgManager/main/scripts/install.sh | VERSION=v1.0.0 bash
```

### From source

Requires a recent stable [Rust](https://www.rust-lang.org/tools/install) toolchain (edition 2024).

```sh
git clone https://github.com/Julius-IX/AppImgManager.git
cd AppImgManager
cargo install --path .
```

The binary is called `app-img`.

### Uninstall

```sh
sudo /usr/local/share/app-img/uninstall.sh
```

This removes the program, not the apps you tracked with it.

---

## Usage

`app-img` has two modes:

```sh
app-img cli <APPIMAGE> [OPTIONS]   # run an action directly
app-img gui [APPIMAGE]             # open the GUI
```

### CLI

```sh
# track a new AppImage (moved to ~/Applications, menu entry + icon added)
app-img cli ~/Downloads/Foo-1.2.3-x86_64.AppImage

# track it and also put a link on the Desktop
app-img cli Foo.AppImage --link

# store it somewhere other than ~/Applications
app-img cli Foo.AppImage --destination ~/Tools

# use your own icon instead of the bundled one
app-img cli Foo.AppImage --icon ~/Pictures/foo.png

# replace a tracked app with a newer version (keeps the Desktop link)
app-img cli ~/Downloads/Foo-1.3.0-x86_64.AppImage --update

# stop tracking an app and delete its files
app-img cli ~/Applications/Foo.AppImage --clean
```

| Option                  | Description                                                                  |
| ----------------------- | ---------------------------------------------------------------------------- |
| `-d`, `--destination`   | Folder to put the AppImage in (must already exist)                           |
| `-u`, `--update`        | Replace the tracked app this AppImage is a new version of, matched by its desktop entry name |
| `-c`, `--clean`         | Stop tracking the app and delete it from the filesystem                      |
| `-i`, `--icon`          | Icon to use instead of the bundled one. On an already tracked app, just swaps its icon |
| `-l`, `--link`          | Create a hard link in the Desktop folder                                     |

`--clean` can't be combined with `--link`, `--update` or `--icon`.

### GUI

```sh
app-img gui
```

The home screen lists every tracked app. From there you can track a new AppImage, or open an app to update it, change its icon, toggle its Desktop link, or delete it.
File and folder pickers are native.

Passing a path opens straight on the "Track new AppImage" screen with the file filled in. This is what the install script's "Open with" entry uses:

```sh
app-img gui ~/Downloads/Foo-1.2.3-x86_64.AppImage
```

### Notes

- Only AppImages that support `--appimage-extract` can have their entry and icon read. On some distros an AppImage won't start without FUSE 2 (`libfuse2`, or `libfuse2t64` on Ubuntu 24.04).
- Desktop links are hard links, so the Desktop folder must be on the same filesystem as the stored AppImage.
- An app whose desktop entry name is already tracked is refused. Use `--update` to replace it.
