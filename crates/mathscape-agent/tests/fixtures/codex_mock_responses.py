# Codex Responses API mock used by tests/codex_live_mock.rs (port, log dir).
# Replies with a shell call unless the last user message contains NOTOOL.
import json, sys, os, itertools, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
LOG = sys.argv[2]; os.makedirs(LOG, exist_ok=True)
cnt = itertools.count(1)
def sse(ev): return f"event: {ev['type']}\ndata: {json.dumps(ev)}\n\n".encode()
class H(BaseHTTPRequestHandler):
    def log_message(self,*a): pass
    def do_GET(self):
        n=next(cnt); open(f"{LOG}/{n:03d}_GET.txt","w").write(self.path)
        if 'models' in self.path:
            b=json.dumps({"models":[]}).encode()
            self.send_response(200); self.send_header('content-type','application/json'); self.send_header('content-length',str(len(b))); self.end_headers(); self.wfile.write(b)
        else:
            self.send_response(404); self.end_headers()
    def do_POST(self):
        n=next(cnt); l=int(self.headers.get('content-length',0)); raw=self.rfile.read(l)
        enc=self.headers.get('content-encoding')
        if enc=='zstd':
            import subprocess; raw=subprocess.run(['zstd','-dc'],input=raw,capture_output=True).stdout
        try: body=json.loads(raw)
        except Exception: body={"_raw":raw[:2000].decode('latin1')}
        json.dump({"path":self.path,"headers":dict(self.headers),"body":body},open(f"{LOG}/{n:03d}_POST.json","w"),indent=1)
        if not self.path.endswith('/responses'):
            self.send_response(404); self.end_headers(); return
        inp=body.get('input',[])
        tools=[t.get('name') or t.get('type') for t in body.get('tools',[])]
        last=inp[-1] if inp else {}
        rid=f"resp_{n}"
        items=[]
        lastuser=''
        for it in inp:
            if it.get('type')=='message' and it.get('role')=='user':
                c=it.get('content',[]); lastuser=' '.join(x.get('text','') for x in c if isinstance(x,dict))
        if last.get('type')=='function_call_output' or 'NOTOOL' in lastuser:
            items.append({"type":"message","role":"assistant","id":f"msg_{n}","status":"completed","content":[{"type":"output_text","text":f"MOCK_REPLY_{n} seen_input_items={len(inp)}","annotations":[]}]})
        else:
            tool = 'exec_command' if 'exec_command' in tools else ('shell_command' if 'shell_command' in tools else 'shell')
            if tool=='exec_command': args={"cmd":"pwd; echo MARKER_TOOL_OUTPUT","yield_time_ms":2000}
            elif tool=='shell_command': args={"command":"pwd; echo MARKER_TOOL_OUTPUT"}
            else: args={"command":["bash","-lc","pwd; echo MARKER_TOOL_OUTPUT"]}
            items.append({"type":"function_call","id":f"fc_{n}","call_id":f"call_{n}","name":tool,"arguments":json.dumps(args),"status":"completed"})
        self.send_response(200); self.send_header('content-type','text/event-stream'); self.end_headers()
        self.wfile.write(sse({"type":"response.created","response":{"id":rid}}))
        for it in items:
            self.wfile.write(sse({"type":"response.output_item.added","output_index":0,"item":it}))
            self.wfile.write(sse({"type":"response.output_item.done","output_index":0,"item":it}))
        self.wfile.write(sse({"type":"response.completed","response":{"id":rid,"usage":{"input_tokens":100,"input_tokens_details":{"cached_tokens":0},"output_tokens":10,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":110}}}))
        self.wfile.flush()
ThreadingHTTPServer(('127.0.0.1',int(sys.argv[1])),H).serve_forever()
