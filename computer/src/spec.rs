//! Every command form an agent may send, in one table. The MCP tool descriptions, the `help`
//! read, the installed protocol reference, and the grammar tests are all generated from it, so
//! they cannot drift from each other. The parser in `command.rs` is what actually accepts a form;
//! a test parses every example here.

pub const READ_TOOL: &str = "android_read";
pub const ACT_TOOL: &str = "android_act";

/// Sent once at `initialize`; kept to what an agent cannot learn from the tool descriptions.
pub const INSTRUCTIONS: &str = "Controls one Android device. Act on a name you expect; read `screen` only if the state is unknown. \
Rows are names to quote, with a tag for non-buttons: `sw Wi-Fi=on`, `edit Email=\"x\"`; `Save#2` means `tap \"Save\" number 2`; `e7` has no label: tap e7. \
Every action returns the new screen or only what `Changed:`. stale: retry. partial or unknown: read before acting again, never repeat. \
Ask first: purchases, messages, account or security changes, deletes, camera, microphone, location, recording.";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Read,
    Act,
}

pub struct Form {
    pub tool: Tool,
    /// Short forms are listed in the tool description; the rest are reached through `help`.
    pub hot: bool,
    pub usage: &'static str,
    /// Concrete commands the parser must accept.
    pub examples: &'static [&'static str],
}

const fn read(hot: bool, usage: &'static str, examples: &'static [&'static str]) -> Form {
    Form { tool: Tool::Read, hot, usage, examples }
}

const fn act(hot: bool, usage: &'static str, examples: &'static [&'static str]) -> Form {
    Form { tool: Tool::Act, hot, usage, examples }
}

pub const FORMS: &[Form] = &[
    read(true, "status", &["status"]),
    read(true, r#"screen [full|changes|matching "TEXT"]"#, &["screen", "screen full", "screen changes", r#"screen matching "VPN""#]),
    read(true, r#"find "TEXT""#, &[r#"find "airplane""#]),
    read(true, "page [text [matching \"TEXT\"]]", &["page", "page text", r#"page text matching "Example""#]),
    read(true, "notifications", &["notifications"]),
    read(false, "browser tabs", &["browser tabs"]),
    read(false, "location | capabilities", &["location", "capabilities"]),
    read(false, r#"image hash "NAME" | image difference "A" and "B""#, &[r#"image hash "screen""#, r#"image difference "screen" and "photo""#]),
    act(true, r#"tap "LABEL" [number N] | tap eN"#, &[r#"tap "Wi-Fi""#, r#"tap "Save" number 2"#, "tap e12"]),
    act(true, r#"toggle "LABEL""#, &[r#"toggle "Wi-Fi""#]),
    act(false, r#"set "LABEL" on|off"#, &[r#"set "Wi-Fi" on"#, r#"set "Wi-Fi" off"#]),
    act(true, r#"type "TEXT" in "FIELD""#, &[r#"type "hello" in "Message""#]),
    act(
        true,
        r#"scroll up|down|left|right [in "AREA"] [until "TEXT"]"#,
        &["scroll down", r#"scroll down in "List""#, r#"scroll down until "About""#, r#"scroll down in "List" until "About""#],
    ),
    act(true, "press back|home|recents|notifications|enter", &["press back", "press enter"]),
    act(true, r#"open app "NAME""#, &[r#"open app "Settings""#]),
    act(true, r#"open link "URL""#, &[r#"open link "https://example.com""#]),
    act(true, r#"wait for text "TEXT" up to N seconds"#, &[r#"wait for text "Done" up to 5 seconds"#]),
    act(true, r#"then find "TEXT"|screen (a look ends the chain)"#, &[r#"tap "Save" then find "Saved""#, "tap \"Save\" then screen"]),
    act(true, r#"verify text "TEXT" exists"#, &[r#"verify text "Saved" exists"#]),
    act(
        true,
        r#"page open "URL" | page click "LABEL" | page type "TEXT" in "FIELD""#,
        &[r#"page open "https://example.com""#, r#"page click "More""#, r#"page type "hi" in "Search""#],
    ),
    act(
        false,
        r#"click "LABEL" | turn on|off "LABEL" | go back (also: curly quotes, a screen row such as "sw Wi-Fi=on" as LABEL)"#,
        &[r#"click "Wi-Fi""#, r#"turn on "Wi-Fi""#, "go back"],
    ),
    act(false, r#"hold "LABEL""#, &[r#"hold "Photo""#]),
    act(false, r#"open setting "wifi|bluetooth|display|sound|notifications|apps|battery|accessibility|date and time|developer options""#, &[r#"open setting "wifi""#]),
    act(false, r#"wait for "LABEL" up to N seconds | wait for screen change up to N seconds"#, &[r#"wait for "Next" up to 5 seconds"#, "wait for screen change up to 5 seconds"]),
    act(false, r#"verify "LABEL" exists | verify "LABEL" is gone"#, &[r#"verify "Next" exists"#, r#"verify "Next" is gone"#]),
    act(
        false,
        r#"page press "Enter|Tab|Escape|Backspace|ArrowUp|ArrowDown" | page scroll PIXELS | page wait for text "TEXT" up to N seconds | page focus "LABEL" | page back|forward|reload|screenshot"#,
        &[r#"page press "Enter""#, "page scroll 600", r#"page wait for text "Done" up to 5 seconds"#, r#"page focus "Email""#, "page back", "page screenshot"],
    ),
    act(false, r#"new tab "URL" | select tab "TITLE" | close tab "TITLE""#, &[r#"new tab "https://example.com""#, r#"select tab "Docs""#, r#"close tab "Docs""#]),
    act(
        false,
        "capture screen | take rear|front camera photo [at W by H] | record microphone|screen for N seconds",
        &["capture screen", "take rear camera photo", "record screen for 5 seconds"],
    ),
    act(
        false,
        r#"open notification "TITLE" | dismiss notification "TITLE" | run notification action ["ACTION" on] "TITLE""#,
        &[r#"open notification "Messages""#, r#"dismiss notification "Messages""#, r#"run notification action "Messages""#, r#"run notification action "Archive" on "Messages""#],
    ),
    act(
        false,
        "tap point X Y | swipe from X1 Y1 to X2 Y2 over MS milliseconds (coordinates on the attached screen image)",
        &["tap point 100 200", "swipe from 500 1500 to 500 500 over 300 milliseconds"],
    ),
    act(false, r#"crop image "NAME" from X Y with size W by H"#, &[r#"crop image "screen" from 0 0 with size 100 by 100"#]),
];

fn usages(tool: Tool, hot_only: bool) -> Vec<&'static str> {
    FORMS.iter().filter(|form| form.tool == tool && (form.hot || !hot_only)).map(|form| form.usage).collect()
}

pub fn read_description() -> String {
    format!("Look at the device without changing it: {}. `help` lists every command.", usages(Tool::Read, true).join(" | "))
}

pub fn act_description() -> String {
    format!(
        "Change the device. Join up to 16 steps with `then`. Quote names exactly as `screen` shows them.\n{}\nOther commands: {READ_TOOL} help",
        usages(Tool::Act, true).join(" | ")
    )
}

fn listing(tool: Tool) -> String {
    usages(tool, false).iter().map(|usage| format!("- {usage}\n")).collect()
}

/// These are handled before the parser (`session.rs`), so they are listed here and not in `FORMS`.
const EXTRAS: &str = "Routines (replay a command that worked, in one call):\n\
- save routine \"NAME\" from last [with \"VALUE\" ...] (each VALUE must be quoted in the last command; it becomes a parameter)\n\
- run routine \"NAME\" [with \"VALUE\" ...] (runs like any command, so a changed screen is a miss)\n\
Secrets and waiting:\n\
- type secret \"NAME\" in \"FIELD\" (first step of a command; only a password field of an app the secret allows)\n\
- ask phone \"QUESTION\" then STEPS (a person answers on the phone; ask before payments, messages, deletes; nothing runs if denied or the screen changed)\n\
- push file \"NAME\" | pull file \"NAME\" (the phone's Downloads; needs files.dir in policy.toml; ask the user first)\n\
- wait for notification [from \"APP\"] up to N seconds (N is 1-120)\n\
- android_read routines | secrets | policy | audit | code from notification \"APP\"\n";

/// The full command list, returned by `help`.
pub fn help_text() -> String {
    format!(
        "{READ_TOOL} (never changes the device):\n{}\n{ACT_TOOL} (join up to 16 steps with then; N is 0-30; ask the user before camera, microphone, recording, notifications):\n{}\n{EXTRAS}",
        listing(Tool::Read),
        listing(Tool::Act)
    )
}

/// The installed protocol reference, `skills/android-use/references/protocol.md`.
pub fn protocol_markdown() -> String {
    format!(
        "# Commands\n\nGenerated from `computer/src/spec.rs`; edit the table, then run `cargo xtask docs --write`.\n\n## {READ_TOOL}\n\n{}\n## {ACT_TOOL}\n\nJoin up to 16 steps with `then`. Quote names as `screen` shows them; `Name#2` is `\"Name\" number 2`. N is 0-30. `tap point` and `swipe` use the attached image's pixels, only after a label is missing.\n\n{}",
        listing(Tool::Read),
        listing(Tool::Act)
    )
}
