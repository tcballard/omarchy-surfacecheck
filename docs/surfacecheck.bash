# Bash completion for the bounded SurfaceCheck command surface.
_surfacecheck_complete() {
    local cur="${COMP_WORDS[COMP_CWORD]}"
    local commands="status service capture review compare select-before-after annotate export handoff cancel"
    local options="--json --session --note --alias --agent --consent-local --consent-external"
    if [[ "${COMP_WORDS[*]}" == *"--json"* ]]; then
        options="--session --note --alias --agent --consent-local --consent-external"
    fi
    COMPREPLY=( $(compgen -W "$commands $options" -- "$cur") )
}
complete -F _surfacecheck_complete surfacecheck
