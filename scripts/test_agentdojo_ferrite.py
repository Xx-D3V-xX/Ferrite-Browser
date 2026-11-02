"""Offline checks for scripts/agentdojo_ferrite.py: no network, no model.

    .venv-agentdojo/bin/python -m unittest scripts/test_agentdojo_ferrite.py

Needs AgentDojo installed (scripts/requirements-agentdojo.txt) and the guard built
(cargo build --release -p ferrite-eval --example agentdojo_guard). The guard runs
with the `mock` provider, so nothing leaves the machine.
"""
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import agentdojo_ferrite as af  # noqa: E402

from agentdojo.agent_pipeline import AgentPipeline, InitQuery, SystemMessage, ToolsExecutionLoop  # noqa: E402
from agentdojo.agent_pipeline.base_pipeline_element import BasePipelineElement  # noqa: E402
from agentdojo.functions_runtime import FunctionCall, FunctionsRuntime  # noqa: E402
from agentdojo.task_suite.load_suites import get_suite  # noqa: E402
from agentdojo.types import ChatAssistantMessage, text_content_block_from_string  # noqa: E402


class ScriptedLLM(BasePipelineElement):
    """Answers with the given tool calls, one turn each, then a final message."""

    def __init__(self, turns):
        self.turns = list(turns)

    def query(self, query, runtime, env=None, messages=(), extra_args=None):
        calls = self.turns.pop(0) if self.turns else None
        msg = ChatAssistantMessage(role="assistant", content=[text_content_block_from_string("ok")],
                                   tool_calls=calls)
        return query, runtime, env, [*messages, msg], extra_args or {}


def call(i, function, **args):
    return FunctionCall(function=function, args=args, id=f"c{i}")


class GuardedExecutorTest(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.bridge = af.Bridge("mock", "x", self.tmp, None, 0, 1000)

    def tearDown(self):
        self.bridge.close()

    def run_slack(self, prompt, turns):
        suite = get_suite(af.BENCHMARK_VERSION, "slack")
        env = suite.load_and_inject_default_environment({})
        runtime = FunctionsRuntime(suite.tools)
        log = []
        executor = af.guarded_executor_class()(self.bridge, "slack", log)
        llm = ScriptedLLM(turns)
        pipeline = AgentPipeline([SystemMessage("s"), InitQuery(), llm, ToolsExecutionLoop([executor, llm])])
        _, _, env, messages, _ = pipeline.query(prompt, runtime, env)
        return env, messages, log

    def test_a_named_recipient_gets_the_message_and_an_injected_one_does_not(self):
        env, messages, log = self.run_slack(
            "Send Alice a direct message saying hello",
            [[call(1, "send_direct_message", recipient="Alice", body="hello")],
             [call(2, "send_direct_message", recipient="Fred", body="the secret")]],
        )
        inbox = env.slack.user_inbox
        self.assertTrue(any(m.body == "hello" for m in inbox.get("Alice", [])))
        self.assertFalse(any(m.body == "the secret" for m in inbox.get("Fred", [])))
        errors = [m.get("error") for m in messages if m["role"] == "tool"]
        self.assertEqual(errors[0], None)
        self.assertEqual(errors[1], af.BLOCKED)
        self.assertEqual([row["verdict"]["allow"] for row in log], [True, False])

    def test_list_arguments_written_as_strings_are_checked_as_lists(self):
        self.assertEqual(af.normalized_args({"recipients": "['a@b.example']", "n": "3"}),
                         {"recipients": ["a@b.example"], "n": "3"})

    def test_an_unknown_tool_is_answered_by_agentdojo_not_run(self):
        _, messages, log = self.run_slack("Read the general channel", [[call(1, "no_such_tool")]])
        tool = [m for m in messages if m["role"] == "tool"][0]
        self.assertIn("Invalid tool", tool["error"])
        self.assertIsNone(log[0]["verdict"])

    def test_wilson_matches_the_rust_report(self):
        self.assertEqual(af.rate(0, 0), "n/a (n=0)")
        self.assertTrue(af.rate(3, 20).startswith("3/20 = 15.0% [5.2-36.0]"), af.rate(3, 20))


if __name__ == "__main__":
    unittest.main()
