# hishell

Toolkit-agnostic core for the [Hishell](../hishell-qt) file manager.

`hishell` owns configuration, locations, view layout, selection, actions, file
operations, navigation plans, and history. It contains no Qt, Slint, or KDE
code; frontends render the state it produces.

`hishell-qt` is the current reference frontend. A Slint frontend is planned.

## Building

```sh
cargo build
```

The crate has no system dependencies.
