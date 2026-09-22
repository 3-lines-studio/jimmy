#!/bin/bash
# Corre la cadena entera de jimmy con dos mentiras: un Telegram falso y un
# modelo falso. Todo lo demás es real: el binario, el transporte, el worker en
# su proceso, el log de la conversación, el HTTP y el SSE.
#
#   tests/e2e/run.sh [ruta al binario]     (por defecto target/debug/jimmy)
#
# Necesita el override TELEGRAM_API_BASE del transporte, que existe justamente
# para esto. Falla con exit 1 y dice qué no cuadró.

set -u

BIN=${1:-target/debug/jimmy}
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
FAIL=0
PIDS=()

cleanup() {
    for pid in "${PIDS[@]:-}"; do kill "$pid" 2>/dev/null; done
    sleep 1
    for pid in "${PIDS[@]:-}"; do kill -9 "$pid" 2>/dev/null; done
}
trap cleanup EXIT

say() { printf '\n=== %s\n' "$*"; }
check() {
    if [ "$2" = "0" ]; then
        printf 'ok   %s\n' "$1"
    else
        printf 'FALLA %s\n' "$1"
        FAIL=1
    fi
}

[ -x "$BIN" ] || { echo "no encuentro el binario: $BIN"; exit 1; }
BIN=$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")

say "preparo el root"
mkdir -p "$WORK/root/prompts" "$WORK/root/workspace/projects/ken"
echo "sos jimmy, un ayudante." >"$WORK/root/prompts/jimmy.md"

free_port() {
    python3 -c "import socket;s=socket.socket();s.bind(('127.0.0.1',0));print(s.getsockname()[1])"
}

say "levanto el Telegram falso"
python3 "$HERE/telegram.py" 0 "$WORK/inbox.jsonl" >"$WORK/telegram.log" 2>&1 &
PIDS+=($!)
python3 "$HERE/model.py" 0 >"$WORK/model.log" 2>&1 &
PIDS+=($!)
sleep 2
TG_PORT=$(rg -o 'PORT [0-9]+' "$WORK/telegram.log" | rg -o '[0-9]+' | head -1)
MODEL_PORT=$(rg -o 'PORT [0-9]+' "$WORK/model.log" | rg -o '[0-9]+' | head -1)
WEB_PORT=$(free_port)
if [ -z "$TG_PORT" ] || [ -z "$MODEL_PORT" ]; then
    echo "no arrancaron los falsos:"
    cat "$WORK/telegram.log" "$WORK/model.log"
    exit 1
fi

say "levanto jimmy (transport=telegram, con la web adentro)"
env -u RESEND_API_KEY -u JIMMY_WEB_FROM \
    TELEGRAM_API_BASE="http://127.0.0.1:$TG_PORT" \
    TELEGRAM_BOT_TOKEN=test \
    TELEGRAM_ALLOWED_USER_IDS=999 \
    JIMMY_ROOT="$WORK/root" \
    JIMMY_WORKSPACE="$WORK/root/workspace" \
    JIMMY_PROMPT=jimmy \
    JIMMY_WEB_PORT="$WEB_PORT" \
    JIMMY_WEB_EMAILS=berti@ejemplo.com JIMMY_WEB_DEV=1 \
    JIMMY_WEB_URL="http://127.0.0.1:$WEB_PORT" \
    AXE_BASE="http://127.0.0.1:$MODEL_PORT/v1" \
    AXE_MODEL=fake \
    OPENAI_API_KEY=test \
    "$BIN" >"$WORK/jimmy.log" 2>&1 &
JIMMY_PID=$!
PIDS+=($JIMMY_PID)
sleep 3

say "primer mensaje, sin nadie mirando"
printf '%s\n' '{"update_id":1,"message":{"message_id":7,"chat":{"id":999},"from":{"id":999,"is_bot":false},"text":"decime hola"}}' >"$WORK/inbox.jsonl"
sleep 9

say "el estado y el stream, con la conversación ya existiendo"
python3 "$HERE/sse.py" "$WEB_PORT" berti@ejemplo.com 999 25 >"$WORK/sse.log" 2>&1 &
SSE_PID=$!
PIDS+=($SSE_PID)
sleep 2

say "segundo mensaje, con el stream enganchado"
printf '%s\n' '{"update_id":2,"message":{"message_id":8,"chat":{"id":999},"from":{"id":999,"is_bot":false},"text":"y ahora?"}}' >>"$WORK/inbox.jsonl"
wait $SSE_PID 2>/dev/null

say "lo que le llegó a Telegram"
cat "$WORK/telegram.log"
rg -q 'EDIT .*La tool devolvió hola\.' "$WORK/telegram.log"
check "el turno terminó y la respuesta volvió a Telegram" $?

rg -q '"event":"tool_start".*"name":"bash"' "$WORK/root/chats/999/conversation.jsonl"
check "el log de la conversación tiene la tool" $?
rg -q '"event":"done"' "$WORK/root/chats/999/conversation.jsonl"
check "el log de la conversación tiene el cierre" $?

say "el stream de la web"
cat "$WORK/sse.log"
rg -q 'STATE .*"key": "999".*"read_only": true' "$WORK/sse.log"
check "el estado lista la conversación de Telegram como solo lectura" $?
rg -q 'SSE data: .*synced' "$WORK/sse.log"
check "el stream marca dónde termina el backlog" $?
rg -qF '"event":"user","text":"y ahora?"' "$WORK/sse.log"
check "el segundo mensaje llegó en vivo por el stream" $?

say "el comando de compactar, por Telegram"
printf '%s\n' '{"update_id":3,"message":{"message_id":9,"chat":{"id":999},"from":{"id":999,"is_bot":false},"text":"/compact"}}' >>"$WORK/inbox.jsonl"
sleep 14
rg -q 'compactado' "$WORK/telegram.log"
check "el compactado respondió por Telegram" $?
rg -q '"type":"compaction"' "$WORK/root/chats/999/transcript.jsonl"
check "el transcript tiene la compactación" $?

say "el apagado, como en un redeploy"
kill -TERM $JIMMY_PID
sleep 3
kill -0 $JIMMY_PID 2>/dev/null
[ $? -ne 0 ] && check "jimmy salió con SIGTERM" 0 || check "jimmy salió con SIGTERM" 1
LEFT=$(pgrep -f "^$BIN worker .*--cwd $WORK/root/workspace" | wc -l)
[ "$LEFT" = "0" ] && check "no quedaron workers huérfanos" 0 || check "no quedaron workers huérfanos" 1

say "logs para mirar si algo falló"
echo "jimmy:    $WORK/jimmy.log"
echo "telegram: $WORK/telegram.log"
echo "modelo:   $WORK/model.log"

if [ "$FAIL" = "0" ]; then
    say "TODO OK"
else
    say "ALGO FALLÓ"
fi
exit $FAIL
