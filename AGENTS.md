# Codex work in this terminal project

- Keep the Codex CLI and its parent terminal session running while exploring or testing this repository. Never send OS-wide keystrokes, close terminal windows or tabs, or signal Codex processes.
- Do not launch the Shift/Runway GUI or its `--script` driver from the terminal that is hosting Codex. The script driver creates a real window, and close-key tests can affect the user's active session. Use builds and unit tests for routine checks; run UI tests only in an isolated display/session.
- Do not use `kill`, `pkill`, `killall`, `fg`, or terminal job-control commands against the user's Codex or parent shell processes. Report suspended jobs so the user can recover them from the owning shell.
