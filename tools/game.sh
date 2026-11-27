# Sourced by the shell tools: picks the game from the first argument or $GAME (a folder in
# games/) and sets GAME, GAME_DIR, WORK, BUILD and PACKAGE (see tools/game.py).
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [ -z "${GAME:-}" ] && [ $# -gt 0 ] && [ -f "$ROOT/games/$1/game.json" ]; then
    GAME="$1"; shift
fi
if [ -z "${GAME:-}" ] || [ ! -f "$ROOT/games/$GAME/game.json" ]; then
    echo "$(basename "$0"): give the game as first argument or GAME=<id> ($(python3 "$ROOT/tools/game.py" | tr '\n' ' '))" >&2
    exit 2
fi
eval "$(python3 "$ROOT/tools/game.py" --shell "$GAME")"
