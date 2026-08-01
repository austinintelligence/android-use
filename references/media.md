# Media

`ss` uses direct byte capture (`adb exec-out screencap -p`) into a non-clobbering PNG. `screen record N` is a finite, no-window scrcpy recording with device audio enabled by default. `mirror N`, `cam view N`, and scrcpy recording paths use verified official scrcpy 4.1; AU does not create Windows virtual camera or microphone devices. Install or verify the pinned archive with `scripts/install-scrcpy.ps1`.

The helper provides camera list/JPEG/finite H.264 MP4/multipart-MJPEG, and microphone PCM16/WAV/finite PCM stream output. Camera and microphone commands require explicit confirmation. Media watchdogs stop on missing host heartbeat, connection loss, helper stop, explicit close, or finite deadline. Normal output is artifact metadata only; `--binary` is mandatory for pipe bytes.
