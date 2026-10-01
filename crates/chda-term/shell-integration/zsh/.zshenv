# chda shell integration for zsh. Sourced by zsh because chda points ZDOTDIR
# at this directory; it restores the user's ZDOTDIR, loads the user's own
# .zshenv, and arranges for chda-integration to run in interactive shells.

if [[ -n "${CHDA_ZSH_ZDOTDIR+X}" ]]; then
    'builtin' 'export' ZDOTDIR="$CHDA_ZSH_ZDOTDIR"
    'builtin' 'unset' 'CHDA_ZSH_ZDOTDIR'
else
    'builtin' 'unset' 'ZDOTDIR'
fi

{
    'builtin' 'typeset' _chda_file="${ZDOTDIR-$HOME}/.zshenv"
    [[ ! -r "$_chda_file" ]] || 'builtin' 'source' '--' "$_chda_file"
} always {
    if [[ -o 'interactive' ]]; then
        'builtin' 'typeset' _chda_file="${${(%):-%x}:A:h}/chda-integration"
        [[ ! -r "$_chda_file" ]] || 'builtin' 'source' '--' "$_chda_file"
    fi
    'builtin' 'unset' '_chda_file'
}
