# gyeol

결 (*gyeol*) — a native UI toolkit for Rust, written from scratch on
[winit](https://github.com/rust-windowing/winit), [wgpu](https://wgpu.rs) and
[cosmic-text](https://github.com/pop-os/cosmic-text).

Work in progress; nothing here is stable or published yet.

```sh
cargo run --example hello        # rounded rectangles and text
cargo run --example textinput    # text fields: mouse, keyboard, Korean IME composition
cargo run --example settings     # declarative API: flexbox layout, hover, click (try --dark --editor)
cargo run --example filelist     # 100,000-row virtualized list: scroll, hover, select
cargo test --lib --examples
```

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.
