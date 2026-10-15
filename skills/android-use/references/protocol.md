# Commands

Generated from `computer/src/spec.rs`; edit the table, then run `cargo xtask docs --write`.

## android_read

- status
- screen [full|changes|matching "TEXT"]
- find "TEXT"
- page [text [matching "TEXT"]]
- notifications
- browser tabs
- location | capabilities
- image hash "NAME" | image difference "A" and "B"

## android_act

Join up to 16 steps with `then`. Quote names as `screen` shows them; `Name#2` is `"Name" number 2`. N is 0-30. `tap point` and `swipe` use the attached image's pixels, only after a label is missing.

- tap "LABEL" [number N] | tap eN
- toggle "LABEL"
- set "LABEL" on|off
- type "TEXT" in "FIELD"
- scroll up|down|left|right [in "AREA"] [until "TEXT"]
- press back|home|recents|notifications|enter
- open app "NAME"
- open link "URL"
- wait for text "TEXT" up to N seconds
- then find "TEXT"|screen (a look ends the chain)
- verify text "TEXT" exists
- page open "URL" | page click "LABEL" | page type "TEXT" in "FIELD"
- click "LABEL" | turn on|off "LABEL" | go back (also: curly quotes, a screen row such as "sw Wi-Fi=on" as LABEL)
- hold "LABEL"
- open setting "wifi|bluetooth|display|sound|notifications|apps|battery|accessibility|date and time|developer options"
- wait for "LABEL" up to N seconds | wait for screen change up to N seconds
- verify "LABEL" exists | verify "LABEL" is gone
- page press "Enter|Tab|Escape|Backspace|ArrowUp|ArrowDown" | page scroll PIXELS | page wait for text "TEXT" up to N seconds | page focus "LABEL" | page back|forward|reload|screenshot
- new tab "URL" | select tab "TITLE" | close tab "TITLE"
- capture screen | take rear|front camera photo [at W by H] | record microphone|screen for N seconds
- open notification "TITLE" | dismiss notification "TITLE" | run notification action ["ACTION" on] "TITLE"
- tap point X Y | swipe from X1 Y1 to X2 Y2 over MS milliseconds (coordinates on the attached screen image)
- crop image "NAME" from X Y with size W by H
