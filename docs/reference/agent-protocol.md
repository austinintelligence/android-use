# Agent protocol

MCP advertises two tools, each taking one string, `command`; `android_read help` lists every command.

- `android_read` never changes the device: `status`, `screen [full|changes|matching "TEXT"]`, `find "TEXT"`, `page [text]`, `notifications`, `help`.
- `android_act` changes it: `tap "LABEL"`, `type "TEXT" in "FIELD"`, `set "Wi-Fi" on` (no tap if already on), `scroll down [until "TEXT"]`, `open app "NAME"`, `wait for text "TEXT" up to 5 seconds`. Join up to 16 steps with `then`. A read (`find`, `screen`) may sit in the chain and ends the result.

## Reading a screen

Act first; read only if the state is unknown.

One row per line: a plain name is a button or text; `sw Wi-Fi=on`, `edit Email="x"`, `chk`, `radio`, `tab`, `link`, `list` mark other controls; `Name: value` shows a short state. Quote the name exactly. `Save#2` is the second duplicate: send `tap "Save" number 2`. `e7` is a control with no label: send `tap e7`. `screen full` adds subtitles.

## Results

A successful action says `Done.` (with a note if a label was a close match), then the screen it led to, or only `Changed:` rows (`~ + -`) when it is the screen you last saw. Chains may cross screens: steps after an app, link, or page opens are resolved on what it shows. Results also carry `structuredContent` with `status` and `changed`.

| Result | Meaning | Next move |
| --- | --- | --- |
| `Done.` | Every step ran. | Continue from the screen shown. |
| `No item labeled …` | Not on screen; that step and later ones did not run. | Use a listed name, `eN`, or the attached image. |
| `… matches N items` | Ambiguous at that step; earlier steps ran. | Add `number N`. |
| `The screen changed before anything ran` | `stale`. Nothing ran. | Retry the same command. |
| `Check failed` / `Waited …` | A verify or wait was false. | Read, then decide. |
| `Partly done` | Some steps changed the device, then one failed. | Read and reconcile; never repeat the whole command. |
| `uncertain` | `unknown`: it may have run. | Read before any other change. |
| `locked`, `screen_off`, `helper_killed`, `stopped` | Nothing ran. | Do the step named, resend. |
| `missing the Android permission` | Android denied a capability. | Ask the user to grant it on the device. |

## Images and coordinates

When a screen exposes little text, or you capture it, its image is attached with a numbered box over each control; the text states its size and scale. `tap eN` is the first choice; `tap point X Y` and `swipe from X1 Y1 to X2 Y2 over MS milliseconds` use coordinates on that image, and the result confirms the device pixel hit. Points outside the image are refused. Any screen change ends the image: capture again before using points, and after rotating. Audio and video are saved to a file path.
