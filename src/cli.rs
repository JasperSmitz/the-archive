//! Database administration deliberately does not initialize image storage.
use std::io::Read;
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Serve,
    Migrate,
    SessionsCleanup,
    AccountList,
    AccountCreate(String, bool),
    AccountReset(String, bool),
    AccountDisable(String),
    Orphans(bool),
    DiscordPrint,
    DiscordRegister,
}
pub const USAGE: &str = "Usage: the-archive [serve | migrate | sessions cleanup | account list | account create USER [--password-stdin] | account reset-password USER [--password-stdin] | account disable USER | cleanup-orphans [--apply] | discord commands print | discord commands register]";
pub fn parse(args: &[String]) -> Result<Command, &'static str> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        [] | ["serve"] => Ok(Command::Serve),
        ["migrate"] => Ok(Command::Migrate),
        ["sessions", "cleanup"] => Ok(Command::SessionsCleanup),
        ["account", "list"] => Ok(Command::AccountList),
        ["account", "disable", name] => Ok(Command::AccountDisable((*name).into())),
        ["account", action @ ("create" | "reset-password"), name] => account(action, name, false),
        [
            "account",
            action @ ("create" | "reset-password"),
            name,
            "--password-stdin",
        ] => account(action, name, true),
        ["cleanup-orphans"] => Ok(Command::Orphans(false)),
        ["cleanup-orphans", "--apply"] => Ok(Command::Orphans(true)),
        ["discord", "commands", "print"] => Ok(Command::DiscordPrint),
        ["discord", "commands", "register"] => Ok(Command::DiscordRegister),
        _ => Err(USAGE),
    }
}
fn account(action: &str, name: &str, stdin: bool) -> Result<Command, &'static str> {
    Ok(if action == "create" {
        Command::AccountCreate(name.into(), stdin)
    } else {
        Command::AccountReset(name.into(), stdin)
    })
}
/// Stdin contains exactly one password, optionally followed by a line ending, then EOF.
/// The cap applies before reading arbitrary input; password spaces are never trimmed.
pub fn password_from_reader(reader: impl Read) -> Result<String, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    reader.take(1027).read_to_end(&mut bytes)?;
    if bytes.len() > 1026 {
        return Err("Password input exceeds the bounded stdin allowance".into());
    }
    let mut password = String::from_utf8(bytes).map_err(|_| "Password must be UTF-8")?;
    if password.ends_with('\n') {
        password.pop();
        if password.ends_with('\r') {
            password.pop();
        }
    }
    if password.contains(['\r', '\n']) {
        return Err("Stdin must contain one password line followed by EOF".into());
    }
    crate::app::auth::password(&password)
        .map_err(|_| "Password must contain 12–1,024 UTF-8 bytes without NUL")?;
    Ok(password)
}
pub fn read_password(stdin: bool) -> Result<String, Box<dyn std::error::Error>> {
    if stdin {
        return password_from_reader(std::io::stdin().lock());
    }
    let password = rpassword::prompt_password("Password: ")?;
    let confirmation = rpassword::prompt_password("Confirm password: ")?;
    if password != confirmation {
        return Err("Passwords did not match".into());
    }
    crate::app::auth::password(&password)
        .map_err(|_| "Password must contain 12–1,024 UTF-8 bytes without NUL")?;
    Ok(password)
}
