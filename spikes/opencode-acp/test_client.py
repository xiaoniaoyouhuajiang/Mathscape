import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


CLIENT = Path(__file__).with_name("client.py")
FAKE = r'''
import json, sys
options = [{"id":"model", "currentValue":"fake/model"}]
sid = "session-1"
prompt_id = None
for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    def send(value):
        print(json.dumps({"jsonrpc":"2.0", **value}), flush=True)
    if method == "session/cancel":
        send({"id":prompt_id,"result":{"stopReason":"cancelled"}})
    elif method == "session/prompt":
        prompt_id = message["id"]
        text = message["params"]["prompt"][0]["text"]
        if text == "error":
            send({"id":prompt_id,"error":{"code":-32603,"message":"fixture error"}})
        elif text == "permission":
            send({"id":"approval-1","method":"session/request_permission","params":{"options":[{"kind":"reject_once","optionId":"reject"},{"kind":"allow_once","optionId":"once"}]}})
        elif text != "wait":
            send({"method":"session/update","params":{"sessionId":sid,"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":text}}}})
            send({"id":prompt_id,"result":{"stopReason":"end_turn"}})
    elif "method" not in message:
        send({"id":prompt_id,"result":{"stopReason":"end_turn"}})
    else:
        send({"id":message["id"],"result":{"sessionId":sid,"configOptions":options}})
'''


class ClientTest(unittest.TestCase):
    def invoke(self, prompt, *extra, expected=0):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "raw.jsonl"
            result = subprocess.run([sys.executable, str(CLIENT), "--cwd", directory,
                                     "--log", str(log), "--prompt", prompt,
                                     "--model", "fake/model", *extra,
                                     "--", sys.executable, "-u", "-c", FAKE],
                                    text=True, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, expected, result.stderr)
            messages = [json.loads(line) for line in log.read_text().splitlines()]
            updates = [json.loads(line) for line in Path(str(log) + ".updates.jsonl").read_text().splitlines()]
            return messages, updates, result

    def test_normal_and_empty_updates(self):
        for text in ["hello", ""]:
            with self.subTest(text=text):
                _, updates, result = self.invoke(text, "--load")
                self.assertEqual(updates[0]["params"]["update"]["content"]["text"], text)
                self.assertIn("MODEL load fake/model", result.stdout)
                self.assertIn("MODEL after_prompt_notifications fake/model", result.stdout)

    def test_permission_policies(self):
        for policy, option in [("reject-all", "reject"), ("allow-once", "once")]:
            with self.subTest(policy=policy):
                messages, updates, _ = self.invoke("permission", "--policy", policy)
                reply = next(m["message"] for m in messages if m.get("message", {}).get("id") == "approval-1")
                self.assertEqual(reply["result"]["outcome"]["optionId"], option)
                self.assertEqual(updates, [])

    def test_cancel(self):
        messages, _, result = self.invoke("wait", "--cancel-after", "0.01")
        self.assertTrue(any(m.get("message", {}).get("method") == "session/cancel" for m in messages))
        self.assertIn('"stopReason": "cancelled"', result.stdout)

    def test_rpc_error(self):
        _, _, result = self.invoke("error", expected=1)
        self.assertIn("fixture error", result.stderr)

    def test_model_mismatch(self):
        _, _, result = self.invoke("hello", "--model", "wrong/model", expected=1)
        self.assertIn("Model mismatch at new", result.stderr)


if __name__ == "__main__":
    unittest.main()
