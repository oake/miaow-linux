mod application;
mod backend;
mod call;
mod reaction;
mod reaction_sync;
mod room;
mod ui;

use application::MiaowApplication;

fn main() -> glib::ExitCode {
    // RUST_LOG can raise or narrow logging without a separate debug build.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("miaow=info"))
        .format_timestamp_millis()
        .init();

    let app = MiaowApplication::new();
    app.run()
}
