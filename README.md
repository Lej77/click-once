# click-once

A small tiny little binary to fix undesired mouse double clicks in Windows, written in Rust. Minimal executable with little to no overhead.

In my machine, CPU usage does not exceed 0.15% and RAM usage is ~700kB

## How it works

It basically hijacks a global hook into the Windows's low level mouse thread input queue and rejects mouse releases which happen too quickly right after a mouse down input.

## Run

```bash
./click-once.exe <delay_left_but> <delay_right_but> <logging>
```

`delay`s are in ms and can be adjusted. The default is 30ms for `delay_left_but` and 0 for `delay_right_but` (disabled).

If `logging` (case insensitive) is provided as one of the arguments then a console window will be opened where click information will be printed. (Requires the program to have been compiled with the `logging` Cargo feature.)

## Build

- [Install Rust](https://www.rust-lang.org/tools/install), on Linux or Windows Subsystem for Linux you can do:

  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

- Clone the repo and build with Cargo

  ```bash
  git clone https://github.com/Lej77/click-once
  cd click-once
  cargo build --release
  ```

  - Alternatively build with `tray` and `logging` Cargo features:

    ```bash
    cargo build --release --features=tray,logging
    ```

  - Alternatively use `nightly` Rust to build the `tray` and `logging` Cargo features while minimizing binary size:

    ```powershell
    $env:RUSTFLAGS="-Zlocation-detail=none -Zfmt-debug=none"
    cargo +nightly build --release --features=tray,logging -Z build-std=std,panic_abort -Z build-std-features=panic_immediate_abort,optimize_for_size
    ```

    - This uses some tricks from <https://github.com/johnthagen/min-sized-rust>.

  - Instead of cloning the repo you can [instruct Cargo to install the executable](https://doc.rust-lang.org/cargo/commands/cargo-install.html) directly:

    ```bash
    cargo install --git https://github.com/Lej77/click-once.git --features=tray,logging
    ```
