# Native daemon wire proxy used only by capture regression fixtures.
import json, os, pathlib, signal, socket, subprocess, sys, threading, time
REAL = __REAL__
FIELD = __FIELD__
ROOT = pathlib.Path(__file__).resolve().parent
if sys.argv[1:] != ["daemon"]:
    os.execv(REAL, [REAL] + sys.argv[1:])
child = subprocess.Popen([REAL, "daemon"], stderr=subprocess.PIPE)
signal.signal(signal.SIGINT, lambda sig, frame: child.send_signal(sig))
signal.signal(signal.SIGTERM, lambda sig, frame: child.send_signal(sig))
lock = threading.Lock()
def trace(value):
    with lock, (ROOT / "identity-wire-events").open("a") as stream:
        stream.write(json.dumps(value) + "\n")
def serve(down, upstream):
    up = socket.socket(socket.AF_UNIX)
    up.connect(upstream)
    def requests():
        try:
            for line in down.makefile("rb"):
                message = json.loads(line)
                if message.get("type") == "terminal_control":
                    trace({"request_id": message["data"]["request_id"]})
                up.sendall(line)
        except (OSError, ValueError):
            pass
        try:
            up.shutdown(socket.SHUT_WR)
        except OSError:
            pass
    threading.Thread(target=requests, daemon=True).start()
    trust_done = False
    held = []
    def inject(message):
        data = message["data"]
        trace({"phase": "frame_injected", "field": FIELD, "native_value": data[FIELD]})
        data[FIELD] = "foreign-native-envelope"
        down.sendall((json.dumps(message) + "\n").encode())
    try:
        for line in up.makefile("rb"):
            message = json.loads(line)
            if message.get("type") == "terminal_frame":
                text = "\n".join("".join(cell["text"] for cell in row) for row in message["data"]["frame"]["cells"])
                if "WARNING: Loading development channels" in text:
                    if trust_done:
                        inject(message)
                    else:
                        held.append(message)
                    continue
            down.sendall(line)
            if message.get("type") == "terminal_control_ack" and message.get("data", {}).get("request_id") == "capture-trust-enter":
                trust_done = True
                trace({"phase": "trust_ack_forwarded"})
                time.sleep(.05)
                for pending in held:
                    inject(pending)
                held.clear()
    except (OSError, ValueError):
        pass
    finally:
        up.close()
        down.close()
def accept(listener, upstream):
    while child.poll() is None:
        try:
            down, _ = listener.accept()
        except OSError:
            break
        threading.Thread(target=serve, args=(down, upstream), daemon=True).start()
listener = None
for line in child.stderr:
    if b"agend daemon ready:" in line:
        path = pathlib.Path(os.environ["AGEND_HOME"]) / "run/daemon.sock"
        upstream = str(path.with_name("native.sock"))
        path.rename(upstream)
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(str(path))
        listener.listen()
        threading.Thread(target=accept, args=(listener, upstream), daemon=True).start()
    sys.stderr.buffer.write(line)
    sys.stderr.buffer.flush()
if listener:
    listener.close()
sys.exit(child.wait())
