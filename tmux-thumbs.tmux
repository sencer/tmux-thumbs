#!/usr/bin/env bash

CURRENT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

DEFAULT_THUMBS_KEY=space

THUMBS_KEY="$(tmux show-option -gqv @thumbs-key)"
THUMBS_KEY=${THUMBS_KEY:-$DEFAULT_THUMBS_KEY}

tmux set-option -ag command-alias "thumbs-pick=run-shell -b ${CURRENT_DIR}/tmux-thumbs.sh"

NO_PREFIX="$(tmux show-option -gqv @thumbs-no-prefix)"

if [ "${NO_PREFIX}" = "1" ]; then
  tmux unbind-key "${THUMBS_KEY}" 2>/dev/null || true
  tmux bind-key -n "${THUMBS_KEY}" thumbs-pick
else
  tmux unbind-key -n "${THUMBS_KEY}" 2>/dev/null || true
  tmux bind-key "${THUMBS_KEY}" thumbs-pick
fi
