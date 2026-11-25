@echo off
rem Rift shell integration for the Command Prompt (cmd.exe): OSC 133 block
rem markers and OSC 7 cwd, carried by PROMPT. Rift starts cmd.exe with
rem `/k call` and its path, so the user's AutoRun and PROMPT apply first;
rem safe to run by hand too.
rem
rem   $e]133;D$e\       D: the command that ran is done (no exit code: cmd
rem                        cannot put one in a prompt)
rem   $e]7;file:///$P$e\  OSC 7: the working directory (spaces and backslashes
rem                        as they are; Rift reads them)
rem   $e]133;A$e\       A: the prompt starts
rem   your prompt       the PROMPT already set, else cmd's default $P$G
rem   $e]133;B$e\       B: the prompt ends, input begins
rem
rem cmd has no pre-execution hook, so there is no 133;C: Rift splits blocks at
rem each A and takes the command line from its own editor.

if defined RIFT_CMD_INTEGRATION goto :eof
set "RIFT_CMD_INTEGRATION=1"

set "RIFT_PROMPT=%PROMPT%"
if not defined RIFT_PROMPT set "RIFT_PROMPT=$P$G"
set "PROMPT=$e]133;D$e\$e]7;file:///$P$e\$e]133;A$e\%RIFT_PROMPT%$e]133;B$e\"
set "RIFT_PROMPT="
