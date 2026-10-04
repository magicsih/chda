# chda shell integration for fish: semantic prompt marks (OSC 133) and
# working directory reports (OSC 7). The title is left to fish_title, which
# fish already writes on its own.
#
# Written for chda and MIT licensed like the rest of it.
#
# chda loads this without touching the user's dotfiles by putting its data
# directory first in XDG_DATA_DIRS, so fish reads this file from
# vendor_conf.d at startup. The first step puts XDG_DATA_DIRS back.

# chda passes the original value, empty when it was unset.
if set -q CHDA_FISH_XDG_DATA_DIRS
    if test -n "$CHDA_FISH_XDG_DATA_DIRS"
        set -gx XDG_DATA_DIRS $CHDA_FISH_XDG_DATA_DIRS
    else
        set -e XDG_DATA_DIRS
    end
    set -e CHDA_FISH_XDG_DATA_DIRS
end

# fish 4.0 and later write OSC 133 and OSC 7 themselves (the mark-prompt
# feature); `status test-feature` fails for fish 3, which does not.
if status is-interactive; and not status test-feature mark-prompt 2>/dev/null
    # Before each prompt: report the directory and mark the prompt start
    # (A). The prompt end (B) goes after fish_prompt's output, so the prompt
    # function is wrapped, again whenever the user redefines it.
    function __chda_prompt_start --on-event fish_prompt
        printf '\e]7;file://%s%s\a' $hostname $PWD
        printf '\e]133;A\a'
        if not string match -q '*__chda_user_prompt*' -- (functions fish_prompt)
            functions -e __chda_user_prompt
            functions -c fish_prompt __chda_user_prompt
            function fish_prompt
                __chda_user_prompt
                printf '\e]133;B\a'
            end
        end
    end

    # After the user hits enter: mark where command output starts (C).
    function __chda_preexec --on-event fish_preexec
        printf '\e]133;C\a'
    end

    # After the command: close its output with the exit status (D).
    function __chda_postexec --on-event fish_postexec
        printf '\e]133;D;%s\a' $status
    end
end

# This also runs on fish 4, whose own prompt marking needs no integration.
if status is-interactive; and set -q CHDA_CODEX_TITLE_CONFIG; and not functions -q codex
    function codex --wraps codex
        command codex -c "$CHDA_CODEX_TITLE_CONFIG" $argv
    end
end
