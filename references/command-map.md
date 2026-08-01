# Command map

Global options may precede or follow the command: `-s SERIAL|usb|wifi|wireless|mdns`, `-j`, `-c`, `-w`, `-q`, `--delay MS`, `--timeout MS`, `--out PATH`, `--force`, `--binary`, `--no-daemon`, `--disasm`/`--decode`. `adb` and `sh` are exact raw argv surfaces: everything after the command remains untouched.

| Family | Commands |
| --- | --- |
| Connection | `d`, `u ENDPOINT`, `p HOST:PORT CODE`, `c HOST:PORT`, `dc HOST:PORT`, `st`, `cap`, `doctor` |
| Fast path | `b DSL_OR_FILE`, `pipe`, `x|tape PROGRAM`, `x|tape --disasm PROGRAM`, `daemon start|stop|status|ping` |
| GUI | `t X Y`, `dt X Y`, `lp X Y`, `sw X1 Y1 X2 Y2 [MS]`, `dr X1 Y1 X2 Y2 [MS]`, `tx TEXT`, `k KEY`, `home`, `back`, `recents`, `notify`, `quick`, `wake`, `sleep`, `rot 0|1|2|3`, `ss` |
| Semantic UI | `ui snap [--expanded]`, `ui find SELECTOR`, `ui tap HANDLE_OR_SELECTOR`, `ui long HANDLE_OR_SELECTOR`, `ui set HANDLE_OR_SELECTOR TEXT`, `ui scroll HANDLE_OR_SELECTOR [forward|backward]`, `ui wait SELECTOR [MS]`, `ui assert SELECTOR [MS]`, `ui watch`, `ui global ACTION`, `ui gesture X1 Y1 X2 Y2 [MS]` |
| Vision ladder | `vision inspect`, `vision hash [PNG]`, `vision diff BASE_PNG [THRESHOLD]`, `vision crop X Y W H`, `vision region X Y W H`, `vision check REGION`, `vision clear` |
| Web | `web open URL`, `web tabs`, `web use ID`, `web go URL`, `web click SELECTOR`, `web type TEXT`, `web text [SELECTOR]`, `web eval JS`, `web wait CONDITION [MS]`, `web back`, `web reload`, `web close [ID]`, `web shot` |
| Apps | `app ls`, `app info PACKAGE`, `app start PACKAGE [ACTIVITY]`, `app stop PACKAGE`, `app install APK`, `app uninstall PACKAGE`, `app clear PACKAGE`, `app perm PACKAGE`, `app grant PACKAGE PERMISSION`, `app revoke PACKAGE PERMISSION`, `app intent ACTION OPTIONS` |
| Media | `mirror [SECONDS]`, `screen record [SECONDS]`, `cam list|view|snap|record|pipe`, `mic cap|pipe` |
| Location | `loc status|get|set|clear|route|enable|disable` |
| System | `clip`, `notif ls|watch|open|action|dismiss`, `file`, `prop`, `settings`, `sys`, `log`, `ps`, `fwd`, `rev` |
| Raw | `adb -- ARGS`, `adb -g -- ARGS`, `sh -- ARGS` |

Coordinates accept integer pixels or percentages such as `50%`. `cap -j` reports both helper installation and authenticated protocol availability; an installed legacy helper can therefore be distinguished from a usable current helper. `--out` artifacts never overwrite an existing path unless `--force` is explicit. For argument-preserving invocation from PowerShell, call `au.exe` directly or use `scripts\\au.ps1`.

`au pipe` consumes newline-delimited DSL in one foreground process. Each
non-empty input line is one bounded batch and produces one response immediately
when that line completes; the shell, helper, and CDP sessions remain warm across
lines.
