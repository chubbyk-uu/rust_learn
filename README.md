# Gomoku ggez Example

This repository contains a simple Gomoku (Five-in-a-Row) game written in Rust
using the [`ggez`](https://github.com/ggez/ggez) game framework. The AI opponent
uses a minimax search with heuristics for evaluating the board.

## Building

The project disables `ggez`'s default `audio` and `gamepad` features so that it
can compile without extra system libraries. To build or run the game simply
execute:

```bash
cargo run
```

If building fails due to missing system packages, ensure you have a recent Rust
toolchain installed and that your system has the libraries required by `ggez`
(for example libudev and OpenGL drivers).

# Running

This example opens a graphical window using `ggez`. It requires an X11 or Wayland environment. Running it in a headless environment will fail with an error such as:

```
Failed to initialize any backend! Wayland status: XdgRuntimeDirNotSet X11 status: XOpenDisplayFailed
```
