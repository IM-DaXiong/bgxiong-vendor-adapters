"""Unit tests for Breeze request_map (no weights)."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from request_map import (  # noqa: E402
    RequestMapError,
    apply_direction_to_body,
    effective_fast_flags,
    map_direction_controls,
    map_speech_request,
    parse_execution,
    parse_runtime_options,
)


class TestPlain(unittest.TestCase):
    def test_plain_ok(self) -> None:
        mapped = map_speech_request(
            {"model": "cfg_breeze", "input": "你好，世界。", "mode": "plain"}
        )
        self.assertEqual(mapped["template_name"], "tts_plain")
        self.assertEqual(mapped["upstream_request"]["text"], "你好，世界。")
        self.assertNotIn("instruction", mapped["upstream_request"])
        self.assertEqual(mapped["execution"]["seed"], 42)
        self.assertEqual(mapped["execution"]["guidanceScale"], 1.0)

    def test_plain_rejects_reference(self) -> None:
        with self.assertRaises(RequestMapError):
            map_speech_request(
                {
                    "model": "cfg_breeze",
                    "input": "hi",
                    "mode": "plain",
                    "reference": {"audioPath": "a.wav", "promptText": "hi"},
                }
            )

    def test_plain_rejects_cfg(self) -> None:
        with self.assertRaises(RequestMapError):
            map_speech_request(
                {
                    "model": "cfg_breeze",
                    "input": "hi",
                    "mode": "plain",
                    "execution": {"guidanceScale": 4},
                }
            )


class TestDesign(unittest.TestCase):
    def test_design_ok(self) -> None:
        mapped = map_speech_request(
            {
                "model": "cfg_breeze",
                "input": "Welcome back.",
                "mode": "voice_design",
                "voiceDesign": {"controlPrompt": "A calm female narrator"},
            }
        )
        self.assertEqual(mapped["template_name"], "tts_instruction")
        self.assertEqual(
            mapped["upstream_request"]["instruction"], "A calm female narrator"
        )
        # Instruction is independent of spoken text.
        self.assertEqual(mapped["upstream_request"]["text"], "Welcome back.")

    def test_design_requires_control(self) -> None:
        with self.assertRaises(RequestMapError):
            map_speech_request(
                {"model": "cfg_breeze", "input": "hi", "mode": "voice_design"}
            )

    def test_design_rejects_reference(self) -> None:
        with self.assertRaises(RequestMapError):
            map_speech_request(
                {
                    "model": "cfg_breeze",
                    "input": "hi",
                    "mode": "voice_design",
                    "voiceDesign": {"controlPrompt": "soft"},
                    "reference": {"audioPath": "a.wav", "promptText": "x"},
                }
            )


class TestClone(unittest.TestCase):
    def test_clone_requires_prompt_text(self) -> None:
        with self.assertRaises(RequestMapError):
            map_speech_request(
                {
                    "model": "cfg_breeze",
                    "input": "下一句",
                    "mode": "reference_clone",
                    "reference": {"audioPath": "C:/ref.wav"},
                }
            )

    def test_clone_ok(self) -> None:
        mapped = map_speech_request(
            {
                "model": "cfg_breeze",
                "input": "下一句正文",
                "mode": "reference_clone",
                "reference": {
                    "audioPath": "C:/ref.wav",
                    "promptText": "参考音完整转写",
                },
            }
        )
        self.assertEqual(mapped["template_name"], "ref_clone_tata")
        self.assertEqual(mapped["upstream_request"]["ref_text"], "参考音完整转写")
        self.assertEqual(mapped["upstream_request"]["text"], "下一句正文")
        self.assertNotIn("instruction", mapped["upstream_request"])


class TestDirection(unittest.TestCase):
    def test_style_control_becomes_edit(self) -> None:
        mapped = map_speech_request(
            {
                "model": "cfg_breeze",
                "input": "继续讲故事",
                "mode": "reference_clone",
                "reference": {
                    "audioPath": "C:/ref.wav",
                    "promptText": "转写",
                    "styleControl": "温柔平静",
                },
            }
        )
        self.assertEqual(mapped["template_name"], "ref_edit_tata")
        self.assertEqual(mapped["upstream_request"]["instruction"], "温柔平静")

    def test_dual_source_direction_join(self) -> None:
        joined = map_direction_controls(
            style_control="声音温柔", emotion_text="略带笑意"
        )
        self.assertEqual(joined, "声音温柔，略带笑意")
        mapped = apply_direction_to_body(
            {
                "model": "cfg_breeze",
                "input": "正文",
                "mode": "reference_clone",
                "reference": {"audioPath": "C:/ref.wav", "promptText": "转写"},
            },
            style_control="声音温柔",
            emotion_text="略带笑意",
        )
        self.assertEqual(mapped["template_name"], "ref_edit_tata")
        self.assertEqual(
            mapped["upstream_request"]["instruction"], "声音温柔，略带笑意"
        )


class TestExtraKeysAndForbidden(unittest.TestCase):
    def test_unknown_top_key(self) -> None:
        with self.assertRaises(RequestMapError):
            map_speech_request(
                {"model": "cfg_breeze", "input": "hi", "mode": "plain", "foo": 1}
            )

    def test_voxcpm_inference_forbidden(self) -> None:
        with self.assertRaises(RequestMapError):
            map_speech_request(
                {
                    "model": "cfg_breeze",
                    "input": "hi",
                    "mode": "plain",
                    "inference": {"cfgValue": 2},
                }
            )

    def test_indextts_emotion_forbidden(self) -> None:
        with self.assertRaises(RequestMapError):
            map_speech_request(
                {
                    "model": "cfg_breeze",
                    "input": "hi",
                    "mode": "plain",
                    "emotion": {"mode": "text"},
                }
            )

    def test_unknown_voice_design_key(self) -> None:
        with self.assertRaises(RequestMapError):
            map_speech_request(
                {
                    "model": "cfg_breeze",
                    "input": "hi",
                    "mode": "voice_design",
                    "voiceDesign": {"controlPrompt": "x", "extra": 1},
                }
            )


class TestExecutionBounds(unittest.TestCase):
    def test_defaults(self) -> None:
        ex = parse_execution(None)
        self.assertEqual(ex["delivery"], "complete")
        self.assertEqual(ex["seed"], 42)
        self.assertEqual(ex["maxOutputTokens"], 1500)
        self.assertEqual(ex["maxContextTokens"], 2048)
        self.assertEqual(ex["repetitionPenalty"], 1.1)
        self.assertTrue(ex["doSample"])
        self.assertTrue(ex["depthDecoder"]["doSample"])

    def test_seed_string_rejected(self) -> None:
        with self.assertRaises(RequestMapError):
            parse_execution({"seed": "42"})

    def test_seed_float_rejected(self) -> None:
        with self.assertRaises(RequestMapError):
            parse_execution({"seed": 42.0})

    def test_seed_out_of_range(self) -> None:
        with self.assertRaises(RequestMapError):
            parse_execution({"seed": -1})
        with self.assertRaises(RequestMapError):
            parse_execution({"seed": 4294967296})

    def test_cfg_non_positive(self) -> None:
        with self.assertRaises(RequestMapError):
            parse_execution({"guidanceScale": 0})

    def test_max_tokens_cap(self) -> None:
        with self.assertRaises(RequestMapError):
            parse_execution({"maxOutputTokens": 1501})

    def test_top_p_bounds(self) -> None:
        with self.assertRaises(RequestMapError):
            parse_execution({"topP": 1.5})
        with self.assertRaises(RequestMapError):
            parse_execution({"topP": 0})

    def test_delivery_invalid(self) -> None:
        with self.assertRaises(RequestMapError):
            parse_execution({"delivery": "chunk"})

    def test_bool_must_be_strict(self) -> None:
        with self.assertRaises(RequestMapError):
            parse_execution({"doSample": 1})


class TestRuntimeOptions(unittest.TestCase):
    def test_fast_all_master_true(self) -> None:
        opts = parse_runtime_options({"fastAll": True, "fastCodec": False})
        eff = effective_fast_flags(opts)
        self.assertTrue(all(eff.values()))

    def test_fast_all_master_false(self) -> None:
        opts = parse_runtime_options({"fastAll": False, "fastCodec": True})
        eff = effective_fast_flags(opts)
        self.assertTrue(all(v is False for v in eff.values()))

    def test_fast_all_null_uses_stages(self) -> None:
        opts = parse_runtime_options(
            {"fastAll": None, "fastTextEncoder": True, "fastCodec": False}
        )
        eff = effective_fast_flags(opts)
        self.assertTrue(eff["fastTextEncoder"])
        self.assertFalse(eff["fastCodec"])


class TestTextIntegrity(unittest.TestCase):
    def test_inline_events_preserved(self) -> None:
        text = "Hello <laugh> world 你好【轻笑】"
        mapped = map_speech_request(
            {"model": "cfg_breeze", "input": text, "mode": "plain"}
        )
        self.assertEqual(mapped["upstream_request"]["text"], text)

    def test_whitespace_only_rejected_but_inner_kept(self) -> None:
        with self.assertRaises(RequestMapError):
            map_speech_request({"model": "cfg_breeze", "input": "   ", "mode": "plain"})
        # Leading/trailing spaces on non-empty text are preserved (no rewrite).
        mapped = map_speech_request(
            {"model": "cfg_breeze", "input": "  hello  ", "mode": "plain"}
        )
        self.assertEqual(mapped["upstream_request"]["text"], "  hello  ")




class TestAuthorBridgeSemantics(unittest.TestCase):
    def test_design_allows_cfg_for_instruction(self) -> None:
        mapped = map_speech_request(
            {
                "model": "cfg_breeze",
                "input": "试听一句",
                "mode": "voice_design",
                "voiceDesign": {"controlPrompt": "沉稳男声，稍慢"},
                "execution": {"guidanceScale": 4},
            }
        )
        self.assertEqual(mapped["template_name"], "tts_instruction")
        self.assertEqual(mapped["execution"]["guidanceScale"], 4.0)

    def test_clone_direction_from_adapter_joined_style(self) -> None:
        # Adapter joins styleControl + emotionControl.text with fullwidth comma before send.
        mapped = map_speech_request(
            {
                "model": "cfg_breeze",
                "input": "下一句",
                "mode": "reference_clone",
                "reference": {
                    "audioPath": "C:/tmp_refs/abc.wav",
                    "promptText": "参考音完整转写",
                    "styleControl": "声音温柔，略带笑意",
                },
            }
        )
        self.assertEqual(mapped["template_name"], "ref_edit_tata")
        self.assertEqual(mapped["upstream_request"]["instruction"], "声音温柔，略带笑意")
        self.assertEqual(mapped["audit"]["instruction"], "声音温柔，略带笑意")

    def test_top_level_lang_and_duration_rejected(self) -> None:
        with self.assertRaises(RequestMapError):
            map_speech_request(
                {"model": "cfg_breeze", "input": "hi", "mode": "plain", "lang": "zh-CN"}
            )
        with self.assertRaises(RequestMapError):
            map_speech_request(
                {
                    "model": "cfg_breeze",
                    "input": "hi",
                    "mode": "plain",
                    "durationFactor": 1.2,
                }
            )

    def test_whitespace_style_does_not_force_edit(self) -> None:
        mapped = map_speech_request(
            {
                "model": "cfg_breeze",
                "input": "下一句",
                "mode": "reference_clone",
                "reference": {
                    "audioPath": "C:/tmp_refs/abc.wav",
                    "promptText": "参考音完整转写",
                    "styleControl": "   ",
                },
            }
        )
        self.assertEqual(mapped["template_name"], "ref_clone_tata")
        self.assertNotIn("instruction", mapped["upstream_request"])


if __name__ == "__main__":
    unittest.main()
