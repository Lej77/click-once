# click-once

A small tiny little binary to fix undesired mouse double clicks in Windows, written in Rust. Minimal executable with little to no overhead.

In my machine, CPU usage does not exceed 0.15% and RAM usage is ~700kB

## How it works

It basically hijacks a global hook into the Windows's low level mouse thread input queue and rejects mouse releases which happen too quickly right after a mouse down input.

## Run

```bash
./click-once.exe <delay_left_button> <delay_right_button> <delay_middle_button> <logging>
```

`delay`s are in ms and can be adjusted. The default is 30ms for `delay_left_button` and 0 (disabled) for `delay_right_button` as well as `<delay_middle_button>`.

If the string `logging` (case insensitive) is provided as one of the arguments then a console window will be opened where click information will be printed. (Requires the program to have been compiled with the `logging` Cargo feature.)

If the process exits immediately you can still see logs for invalid arguments by specifying the `logging` argument as the first argument or by setting the `CLICK_ONCE_LOGGING` environment variable to a non-empty string. (Note that the environment variable approach requires compiling with the `tray` or `std` Cargo feature.) You might need to start the program from a terminal so that the log window doesn't close immediately.

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

  Then you can find the executable at `./target/x86_64-pc-windows-msvc/release/click-once.exe`

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

### Cargo features

This project uses [Cargo features](https://doc.rust-lang.org/cargo/reference/features.html) to conditionally compile some code. When all features are disabled the program will only prevent too fast clicks, nothing else.

Having fewer features enabled makes it easier to audit the code since there is less code to read through. It also minimizes the binary size if that is important to you.

#### `logging`

This feature has very little impact on the binary size and allows the program to write information to a console window about what it is doing. It also allows error reporting when something goes wrong. No logging will be preformed at runtime unless it is activated by:

- Passing the `logging` command line argument to the program when it is started.
- The `std` cargo feature was enabled when compiling and the `CLICK_ONCE_LOGGING` environment variable was non-empty when the program was started.
- The `tray` cargo feature was enabled when compiling and the `Toggle Logging` context menu item on the system tray was clicked.

#### `std`

Internal feature that simplifies some code by using the Rust standard library. Increases binary size by quite a bit.

#### `tray`

When compiled with this feature the program will create a tray icon when it is started. This makes it easier to quit the program using the tray context menu (otherwise you would have to kill it with something like the task manager). The tray also makes it easy to see if the program is active. If the `logging` cargo feature is enabled then the tray also allows toggling the console window and showing statistics about how many clicks have been blocked by the program.
