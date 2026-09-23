# Runway shell integration for fish — OSC 133 block markers + OSC 7 cwd.
# Auto-loaded via vendor_conf.d (Runway appends its data dir to
# XDG_DATA_DIRS); safe to source manually too. fish 4 emits some prompt
# marking natively — duplicates are harmless, Runway's parser is idempotent.

status is-interactive; or exit 0
if set -q RUNWAY_SHELL_INTEGRATION
    exit 0
end
set -gx RUNWAY_SHELL_INTEGRATION 1

function __runway_report_cwd --on-variable PWD
    printf '\e]7;file://%s%s\a' (hostname) "$PWD"
end

function __runway_preexec --on-event fish_preexec
    set -g __runway_cmd_ran 1
    printf '\e]133;C\a'
end

function __runway_postexec --on-event fish_postexec
    printf '\e]133;D;%s\a' $status
    set -e __runway_cmd_ran
end

# wrap fish_prompt: A before, B after
functions -c fish_prompt __runway_orig_prompt
function fish_prompt
    printf '\e]133;A\a'
    __runway_orig_prompt
    printf '\e]133;B\a'
end

__runway_report_cwd
