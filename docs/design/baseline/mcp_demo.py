"""Record a short MCP session the way an agent client has it, for the README demo.

Usage: python3 mcp_demo.py PATH_TO_AU [TARGET]
Runs a few commands against `au --target TARGET serve --mcp` (TARGET defaults to au-e2e) and prints each result
with its time. The screenshot from the last step is written to capture.png in the current directory.
"""
import json, subprocess, sys, base64, time, os
p = subprocess.Popen([sys.argv[1], "--target", (sys.argv[2] if len(sys.argv) > 2 else "au-e2e"), "serve", "--mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
n = 0
def rpc(method, params=None):
    global n; n += 1
    p.stdin.write(json.dumps({"jsonrpc":"2.0","id":n,"method":method,"params":params or {}})+"\n"); p.stdin.flush()
    return json.loads(p.stdout.readline())
rpc("initialize", {"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"demo","version":"1"}})
p.stdin.write(json.dumps({"jsonrpc":"2.0","method":"notifications/initialized"})+"\n"); p.stdin.flush()
steps = [("android_act", 'open app "Settings" then tap "Network & internet"'), ("android_act", 'tap "Airplane mode"'), ("android_act", 'tap "Airplane mode"'), ("android_act", "capture screen")]
out = []
for tool, cmd in steps:
    t = time.time()
    r = rpc("tools/call", {"name": tool, "arguments": {"command": cmd}})["result"]
    ms = int((time.time()-t)*1000)
    text = "\n".join(c["text"] for c in r["content"] if c["type"] == "text")
    for c in r["content"]:
        if c["type"] == "image":
            open("capture.png", "wb").write(base64.b64decode(c["data"]))
    out.append((cmd, text, ms))
    print(f"{tool}: {cmd}   [{ms} ms]\n{text}\n")
p.terminate()
