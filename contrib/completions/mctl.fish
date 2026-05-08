# fish completion for mctl
#
# Install:
#   mkdir -p ~/.config/fish/completions
#   cp contrib/completions/mctl.fish ~/.config/fish/completions/mctl.fish
#
# Pulls dispatch action names from `mctl actions --names` so the list
# stays in sync with the compositor binary you have installed.

# Helper: are we on the top-level (no subcommand chosen yet)?
function __mctl_no_subcommand
    set -l tokens (commandline -opc)
    set -l subs dispatch d tags client-tags layout quit reload watch status actions completions help
    for tok in $tokens[2..-1]
        if contains -- $tok $subs
            return 1
        end
    end
    return 0
end

# Helper: which subcommand is currently active?
function __mctl_current_sub
    set -l tokens (commandline -opc)
    set -l subs dispatch d tags client-tags layout quit reload watch status actions completions help
    for tok in $tokens[2..-1]
        if contains -- $tok $subs
            echo $tok
            return 0
        end
    end
end

# Cached action-name list. Refreshed on each shell session.
function __mctl_actions
    if not set -q __mctl_actions_cache
        set -g __mctl_actions_cache (mctl actions --names 2>/dev/null)
    end
    for a in $__mctl_actions_cache
        echo $a
    end
end

# Live output names from `mctl status`.
function __mctl_outputs
    mctl status 2>/dev/null | awk -F'[ =]' '/^output=/{print $2}'
end

# How many positional args has the current subcommand seen so far?
function __mctl_positional_count
    set -l tokens (commandline -opc)
    set -l sub $argv[1]
    set -l seen_sub 0
    set -l count 0
    set -l i 2
    while test $i -le (count $tokens)
        set -l tok $tokens[$i]
        if test $seen_sub -eq 0
            if test $tok = $sub
                set seen_sub 1
            end
        else
            switch $tok
                case '-*'
                case '*'
                    set count (math $count + 1)
            end
        end
        set i (math $i + 1)
    end
    echo $count
end

# ── Top level ────────────────────────────────────────────────────────────────
complete -c mctl -n __mctl_no_subcommand -s h -l help -d 'print help'
complete -c mctl -n __mctl_no_subcommand -s V -l version -d 'print version'
complete -c mctl -n __mctl_no_subcommand -s o -l output -x -a "(__mctl_outputs)" \
    -d 'output to target (default: focused)'

complete -c mctl -n __mctl_no_subcommand -a dispatch     -d 'dispatch a compositor action by name'
complete -c mctl -n __mctl_no_subcommand -a d            -d 'alias for dispatch'
complete -c mctl -n __mctl_no_subcommand -a tags         -d 'set active tagset (bitmask)'
complete -c mctl -n __mctl_no_subcommand -a client-tags  -d 'mutate the focused client tags'
complete -c mctl -n __mctl_no_subcommand -a layout       -d 'set layout by 0-based index'
complete -c mctl -n __mctl_no_subcommand -a quit         -d 'quit the compositor'
complete -c mctl -n __mctl_no_subcommand -a reload       -d 'reload config.conf'
complete -c mctl -n __mctl_no_subcommand -a watch        -d 'stream state updates'
complete -c mctl -n __mctl_no_subcommand -a status       -d 'print current status'
complete -c mctl -n __mctl_no_subcommand -a actions      -d 'list every dispatch action'
complete -c mctl -n __mctl_no_subcommand -a completions  -d 'emit a shell-completion script'
complete -c mctl -n __mctl_no_subcommand -a help         -d 'show subcommand help'

# ── dispatch ─────────────────────────────────────────────────────────────────
# First positional after `dispatch` is the action name.
complete -c mctl -n "__mctl_current_sub | string match -rq 'dispatch|d'; \
                     and test (__mctl_positional_count (__mctl_current_sub)) -eq 0" \
    -x -a "(__mctl_actions)" -d 'dispatch action'

# After `dispatch setlayout`, complete layout names.
complete -c mctl -n "__mctl_current_sub | string match -rq 'dispatch|d'; \
                     and test (__mctl_positional_count (__mctl_current_sub)) -eq 1; \
                     and contains -- 'setlayout' (commandline -opc)" \
    -x -a "tile scroller grid monocle deck center_tile right_tile vertical_tile \
           vertical_scroller vertical_grid vertical_deck tgmix canvas dwindle"

# ── actions ──────────────────────────────────────────────────────────────────
complete -c mctl -n "test (__mctl_current_sub) = actions" -s v -l verbose \
    -d 'print detail / examples'
complete -c mctl -n "test (__mctl_current_sub) = actions" -s g -l group -x \
    -a 'Tag Focus Layout Scroller Window Scratchpad Overview System' \
    -d 'filter to a single group'
complete -c mctl -n "test (__mctl_current_sub) = actions" -l names \
    -d 'flat newline list of every accepted spelling'

# ── completions ──────────────────────────────────────────────────────────────
complete -c mctl -n "test (__mctl_current_sub) = completions" \
    -x -a 'bash zsh fish elvish powershell' -d 'shell to generate for'

# ── tags / client-tags / layout: positional args, no good completion ─────────
complete -c mctl -n "test (__mctl_current_sub) = tags; \
                     and test (__mctl_positional_count tags) -eq 1" \
    -x -a '0 1' -d 'toggle (0=set, 1=toggle)'

# ── help ─────────────────────────────────────────────────────────────────────
complete -c mctl -n "test (__mctl_current_sub) = help" \
    -x -a 'dispatch tags client-tags layout quit reload watch status actions completions'
