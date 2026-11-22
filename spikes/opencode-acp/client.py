#!/usr/bin/env python3
"""Dependency-free, newline JSON-RPC ACP probe; never used by production."""
import argparse
import asyncio
import json
import os
import signal
from pathlib import Path


async def run(args):
    pending = {}
    counter = 0
    current_model = None
    log = open(args.log, "w")
    updates = open(args.log + ".updates.jsonl", "w")
    stderr = open(args.log + ".stderr", "w")
    process = await asyncio.create_subprocess_exec(
        *args.command, stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE, stderr=stderr, cwd=args.cwd,
        start_new_session=True,
    )

    async def send(message):
        log.write(json.dumps({"direction": "send", "message": message}) + "\n")
        log.flush()
        process.stdin.write((json.dumps(message) + "\n").encode())
        await process.stdin.drain()

    async def read():
        nonlocal current_model
        while line := await process.stdout.readline():
            log.write(json.dumps({"direction": "recv", "raw": line.decode().rstrip()}) + "\n")
            log.flush()
            message = json.loads(line)
            if message.get("method") == "session/update":
                updates.write(line.decode())
                updates.flush()
                for option in message["params"]["update"].get("configOptions", []):
                    if option["id"] == "model":
                        current_model = option["currentValue"]
            if "method" in message and "id" in message:
                if message["method"] == "session/request_permission":
                    options = message["params"]["options"]
                    wanted = "allow_once" if args.policy == "allow-once" else "reject_once"
                    option = next((o for o in options if o["kind"] == wanted), None)
                    outcome = {"outcome": "selected", "optionId": option["optionId"]} if option else {"outcome": "cancelled"}
                    await send({"jsonrpc": "2.0", "id": message["id"], "result": {"outcome": outcome}})
                else:
                    await send({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32601, "message": "Unsupported client method"}})
            elif message.get("id") in pending:
                pending.pop(message["id"]).set_result(message)
        for future in pending.values():
            future.set_exception(RuntimeError("ACP stdout closed"))

    async def request(method, params):
        nonlocal counter
        counter += 1
        future = asyncio.get_running_loop().create_future()
        pending[counter] = future
        await send({"jsonrpc": "2.0", "id": counter, "method": method, "params": params})
        result = await asyncio.wait_for(future, args.timeout)
        print(method, json.dumps(result) if "error" in result or method == "session/prompt" else "OK", flush=True)
        if "error" in result:
            raise RuntimeError(result["error"])
        return result["result"]

    reader = asyncio.create_task(read())
    try:
        await request("initialize", {"protocolVersion": 1, "clientCapabilities": {}, "clientInfo": {"name": "mathscape-spike", "version": "0.1"}})
        servers = []
        if args.mathscape:
            servers = [{"name": "mathscape", "command": str(Path(args.mathscape).resolve()), "args": ["mcp", "--workspace", args.cwd, "--round-file", args.round_file], "env": []}]
        session = await request("session/new", {"cwd": args.cwd, "mcpServers": servers})
        sid = session["sessionId"]
        def check_model(result, stage):
            nonlocal current_model
            for option in result.get("configOptions", []):
                if option["id"] == "model":
                    current_model = option["currentValue"]
            print("MODEL", stage, current_model, flush=True)
            log.write(json.dumps({"check": "model", "stage": stage, "expected": args.model, "actual": current_model}) + "\n")
            log.flush()
            if args.model and current_model != args.model:
                raise RuntimeError(f"Model mismatch at {stage}: {current_model} != {args.model}")
        check_model(session, "new")
        if args.load:
            loaded = await request("session/load", {"sessionId": sid, "cwd": args.cwd, "mcpServers": servers})
            check_model(loaded, "load")
        async def cancel():
            await asyncio.sleep(args.cancel_after)
            await send({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": sid}})
        cancellation = asyncio.create_task(cancel()) if args.cancel_after else None
        try:
            await request("session/prompt", {"sessionId": sid, "prompt": [{"type": "text", "text": args.prompt}]})
            check_model({}, "after_prompt_notifications")
            configured = await request("session/set_config_option", {"sessionId": sid, "configId": "model", "value": args.model or current_model})
            check_model(configured, "after_prompt")
        finally:
            if cancellation:
                cancellation.cancel()
    finally:
        if process.returncode is None:
            os.killpg(process.pid, signal.SIGTERM)
        try:
            await asyncio.wait_for(process.wait(), 5)
        except asyncio.TimeoutError:
            os.killpg(process.pid, signal.SIGKILL)
            await process.wait()
        await asyncio.wait_for(reader, 5)
        log.close()
        updates.close()
        stderr.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cwd", required=True)
    parser.add_argument("--log", required=True)
    parser.add_argument("--prompt", required=True)
    parser.add_argument("--model")
    parser.add_argument("--mathscape")
    parser.add_argument("--round-file", default="/nonexistent-round")
    parser.add_argument("--policy", choices=["reject-all", "allow-once"], default="reject-all")
    parser.add_argument("--timeout", type=float, default=180)
    parser.add_argument("--cancel-after", type=float)
    parser.add_argument("--load", action="store_true")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    options = parser.parse_args()
    if options.command[:1] == ["--"]:
        options.command.pop(0)
    if not options.command:
        options.command = ["opencode", "acp"]
    asyncio.run(run(options))
