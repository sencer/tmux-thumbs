#!/usr/bin/env bash

CURRENT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
RELEASE_DIR="${CURRENT_DIR}/target/release"
THUMBS_BINARY="${RELEASE_DIR}/thumbs"
VERSION=$(grep 'version =' "${CURRENT_DIR}/Cargo.toml" | grep -o "\".*\"" | sed 's/"//g')

# 1. Version and existence check (once on startup/reload)
prompt_update() {
  local mode="$1"
  local cmd="cd ${CURRENT_DIR} && bash ./tmux-thumbs-install.sh ${mode} && tmux run-shell ${CURRENT_DIR}/tmux-thumbs.tmux"
  if tmux list-clients &>/dev/null; then
    tmux split-window "${cmd}"
  else
    tmux set-hook -g client-session-changed "split-window '${cmd}'; set-hook -ug client-session-changed"
  fi
  exit
}

if [ ! -f "$THUMBS_BINARY" ]; then
  prompt_update "install"
elif [[ $(${THUMBS_BINARY} --version) != "thumbs ${VERSION}"  ]]; then
  prompt_update "update"
fi

# 2. Cache options to file
USER_NAME=${USER:-$(id -un)}
tmux show -g | grep '@thumbs-' > "/tmp/thumbs-options-${USER_NAME}.txt" || true

# 3. Bind key directly to Rust binary
DEFAULT_THUMBS_KEY=space

THUMBS_KEY="$(tmux show-option -gqv @thumbs-key)"
THUMBS_KEY=${THUMBS_KEY:-$DEFAULT_THUMBS_KEY}

# Define alias to run the Rust binary directly
tmux set-option -ag command-alias "thumbs-pick=run-shell -b \"${CURRENT_DIR}/target/release/tmux-thumbs --dir ${CURRENT_DIR}\""

NO_PREFIX="$(tmux show-option -gqv @thumbs-no-prefix)"

if [ "${NO_PREFIX}" = "1" ]; then
  tmux unbind-key "${THUMBS_KEY}" 2>/dev/null || true
  tmux bind-key -n "${THUMBS_KEY}" thumbs-pick
else
  tmux unbind-key -n "${THUMBS_KEY}" 2>/dev/null || true
  tmux bind-key "${THUMBS_KEY}" thumbs-pick
fi
