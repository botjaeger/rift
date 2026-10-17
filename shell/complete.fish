# Rift completion server for fish (crates/rift-core/src/shell_complete.rs).
#
# Run as `fish [-l] -i -c (cat this)`, after the user's config. Answers requests
# read from stdin: an id, a directory, the command line, a word count n, then
# n+1 words (used by the bash server; skipped here). The line is handed to
# `complete --do-complete` as one argument (data, never run). The reply on
# stdout is
#   \x1e begin <id>
#   \x1f<word>\x1f<description>      (one per match)
#   \x1e end <id>
# Requests hold no control characters; Rift rejects lines that contain any.

printf '\x1eready\n'
while read -l id; and read -l cwd; and read -l line; and read -l n
    string match -qr '^[0-9]+$' -- $n; or break
    # the words are for bash; fish splits the line itself
    for i in (seq 0 $n)
        read -l w; or break
    end
    printf '\x1ebegin %s\n' $id
    if builtin cd -- $cwd 2>/dev/null
        for c in (complete --do-complete=$line 2>/dev/null)
            set -l parts (string split -m 1 \t -- $c)
            printf '\x1f%s\x1f%s\n' $parts[1] "$parts[2]"
        end
    end
    printf '\x1eend %s\n' $id
end
