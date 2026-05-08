# bash completion for mctl
# vim: ft=bash
#
# Install:
#   mkdir -p ~/.local/share/bash-completion/completions
#   cp contrib/completions/mctl.bash ~/.local/share/bash-completion/completions/mctl
#
# This script extends clap's auto-generated subcommand completion with the
# full dispatch-action list and the layout-name list. Both lists pull from
# `mctl actions --names` / the compositor's announced layout list at
# completion time, so they stay in sync when actions are added.

_mctl() {
    local cur prev words cword
    _init_completion -n =: 2>/dev/null || {
        cur="${COMP_WORDS[COMP_CWORD]}"
        prev="${COMP_WORDS[COMP_CWORD - 1]}"
        words=("${COMP_WORDS[@]}")
        cword=$COMP_CWORD
    }

    local subcommands=(
        actions client-tags completions d dispatch help layout quit
        reload status tags watch
    )

    # Find the active subcommand (skip the global `--output OUT` if present).
    local sub=""
    local i=1
    while [ $i -lt "$cword" ]; do
        case "${words[$i]}" in
            -o|--output)
                i=$((i + 2))
                ;;
            -*)
                i=$((i + 1))
                ;;
            *)
                sub="${words[$i]}"
                break
                ;;
        esac
    done

    # Top-level: complete subcommand or --output value.
    if [ -z "$sub" ]; then
        case "$prev" in
            -o|--output)
                # Pull live output names from `mctl status`. Falls back
                # silently if margo isn't running.
                local outputs
                outputs=$(mctl status 2>/dev/null | awk -F'[ =]' '/^output=/{print $2}')
                # shellcheck disable=SC2207
                COMPREPLY=($(compgen -W "$outputs" -- "$cur"))
                return 0
                ;;
        esac
        # shellcheck disable=SC2207
        COMPREPLY=($(compgen -W "${subcommands[*]} -h --help -V --version -o --output" -- "$cur"))
        return 0
    fi

    case "$sub" in
        d|dispatch)
            # First positional after `dispatch` is the action name.
            # Subsequent args are action-dependent and we don't try
            # to type them — most are bitmasks / integers / strings.
            local nargs=0
            local j=$((i + 1))
            while [ $j -lt "$cword" ]; do
                case "${words[$j]}" in
                    -*) ;;
                    *) nargs=$((nargs + 1)) ;;
                esac
                j=$((j + 1))
            done
            if [ "$nargs" -eq 0 ]; then
                # Fetch the action list from the binary. Cached so
                # repeated tab presses don't re-spawn mctl.
                if [ -z "${_MCTL_ACTIONS_CACHE:-}" ]; then
                    _MCTL_ACTIONS_CACHE="$(mctl actions --names 2>/dev/null)"
                fi
                # shellcheck disable=SC2207
                COMPREPLY=($(compgen -W "$_MCTL_ACTIONS_CACHE" -- "$cur"))
            elif [ "$nargs" -eq 1 ]; then
                # If the action is `setlayout`, complete layout names.
                local action="${words[$((i + 1))]}"
                if [ "$action" = "setlayout" ]; then
                    # shellcheck disable=SC2207
                    COMPREPLY=($(compgen -W "tile scroller grid monocle deck \
                                              center_tile right_tile vertical_tile \
                                              vertical_scroller vertical_grid \
                                              vertical_deck tgmix canvas dwindle" -- "$cur"))
                fi
            fi
            return 0
            ;;
        actions)
            # --group <NAME> completion: enumerate group labels.
            case "$prev" in
                -g|--group)
                    # shellcheck disable=SC2207
                    COMPREPLY=($(compgen -W "Tag Focus Layout Scroller Window Scratchpad Overview System" -- "$cur"))
                    return 0
                    ;;
            esac
            # shellcheck disable=SC2207
            COMPREPLY=($(compgen -W "-v --verbose -g --group --names -h --help" -- "$cur"))
            return 0
            ;;
        completions)
            # shellcheck disable=SC2207
            COMPREPLY=($(compgen -W "bash zsh fish elvish powershell" -- "$cur"))
            return 0
            ;;
        layout)
            # No good way to enumerate indices without parsing
            # `mctl status`; just leave it open.
            return 0
            ;;
        tags|client-tags)
            # First arg is a u32 mask, second is `0` or `1`.
            return 0
            ;;
        help)
            # shellcheck disable=SC2207
            COMPREPLY=($(compgen -W "${subcommands[*]}" -- "$cur"))
            return 0
            ;;
    esac
}

complete -F _mctl mctl
