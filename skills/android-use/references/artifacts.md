# Artifact reference

Media, screenshots, captures, and streamed command output belong in AU-owned artifact storage. Agents receive an artifact handle, byte count, SHA-256, MIME type, and bounded access through `android.artifact`. Do not send media bytes into the transcript by default and do not accept arbitrary filesystem paths from a recipe or remote request.
