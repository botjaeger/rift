# Shift shell integration for fish — OSC 133 block markers + OSC 7 cwd.
# Auto-loaded via vendor_conf.d (Shift appends its data dir to
# XDG_DATA_DIRS); safe to source manually too. fish 4 emits some prompt
# marking natively — duplicates are harmless, Shift's parser is idempotent.

status is-interactive; or exit 0
# Shell-local guard (not exported): a nested or exec'd fish must load its own
# hooks, which an exported variable would suppress.
if set -q __shift_loaded
    exit 0
end
set -g __shift_loaded 1

# OSC 7 wants a percent-encoded path: `%`, spaces and control characters
# (BEL/ESC would end or inject sequences) must not travel raw.
function __shift_report_cwd --on-variable PWD
    printf '\e]7;file://%s%s\a' (hostname) (string escape --style=url -- $PWD)
end

function __shift_preexec --on-event fish_preexec
    set -g __shift_cmd_ran 1
    printf '\e]133;C\a'
end

function __shift_postexec --on-event fish_postexec
    printf '\e]133;D;%s\a' $status
    set -e __shift_cmd_ran
end

# Wrap fish_prompt (A before, B after) right before each prompt is drawn, so a
# fish_prompt defined later (config.fish, a prompt framework) is wrapped too.
function __shift_wrap_prompt --on-event fish_prompt
    functions fish_prompt | string match -q '*__shift_orig_prompt*'; and return
    functions -e __shift_orig_prompt
    functions -c fish_prompt __shift_orig_prompt
    function fish_prompt
        printf '\e]133;A\a'
        __shift_orig_prompt
        printf '\e]133;B\a'
    end
end

__shift_report_cwd
