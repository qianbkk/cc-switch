import { describe, expect, it } from "vitest";
import { codexProviderPresets } from "@/config/codexProviderPresets";

// 预填口径（2026-08-15 官方文档盘点）：只给 native Responses 直连预设填
// reasoningLevels，取值 = 厂商官方文档声明的真实差异化档位子集。
// 后端 codex_canonical_efforts 对未知值静默丢弃——预设里的拼写错误不会报错，
// 只会让 Codex 选择器静默少档/错档，所以白名单校验必须在测试层兜住。
const CANONICAL_EFFORTS = [
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
  "ultra",
];

function catalogModel(presetName: string, modelId: string) {
  const preset = codexProviderPresets.find((item) => item.name === presetName);
  expect(preset, `preset ${presetName}`).toBeDefined();
  const model = (preset?.modelCatalog ?? []).find(
    (item) => item.model === modelId,
  );
  expect(model, `${presetName} catalog model ${modelId}`).toBeDefined();
  return model!;
}

describe("Codex preset pre-filled reasoning levels", () => {
  // 每条期望值都对应官方文档证据（见预设文件内注释）；改动任一侧前先核对来源
  const EXPECTED: Array<[string, string, string[]]> = [
    // 火山官方 Codex 接入文档四份一致：low/medium/high
    ["火山 Agent Plan", "ark-code-latest", ["low", "medium", "high"]],
    ["火山 Coding Plan", "ark-code-latest", ["low", "medium", "high"]],
    // 方舟深度思考文档：本模型无限制的通用四档（minimal=关思考直接回答）
    [
      "DouBaoSeed",
      "doubao-seed-2-1-pro-260628",
      ["minimal", "low", "medium", "high"],
    ],
    // 混元官方枚举 low/high；hy3 开源 chat template 对其他值直接 raise
    ["Tencent Hunyuan", "hy3", ["low", "high"]],
    ["Tencent Hunyuan", "hy3-preview", ["low", "high"]],
    // LongCat 无档位可调：全站唯一 effort 证据=官方示例的 high
    ["Longcat", "LongCat-2.0", ["high"]],
    // xAI Reasoning guide 模型级枚举；grok-4.5 不可关思考故无 none
    ["xAI (Grok)", "grok-4.5", ["low", "medium", "high"]],
    ["xAI (Grok) OAuth", "grok-4.5", ["low", "medium", "high"]],
    // 千帆 Token Plan：deepseek-v4-pro/v4-flash 在 thinking+reasoning_effort
    // 双官方清单内（effort 仅 high/max 两档真实深度）；不声明 default=回落
    // max，恰好等于平台对复杂 Agent 类请求的自动行为。glm-5.1 只在 thinking
    // 清单 → 两态
    ["Baidu Qianfan Token Plan", "deepseek-v4-pro", ["none", "high", "max"]],
    ["Baidu Qianfan Token Plan", "deepseek-v4-flash", ["none", "high", "max"]],
    ["Baidu Qianfan Token Plan", "glm-5.1", ["none", "high"]],
    // StepFun 官方两站模型页+reasoning 指南：3.7-flash 三档（默认 medium）、
    // 2603 两档；全系无关思考形态故无 none。effort 下发走后端 per-model
    // 推断（2603=low_high、3.7=passthrough）
    ["StepFun", "step-3.7-flash", ["low", "medium", "high"]],
    ["StepFun", "step-3.5-flash-2603", ["low", "high"]],
    ["StepFun en", "step-3.7-flash", ["low", "medium", "high"]],
    ["StepFun en", "step-3.5-flash-2603", ["low", "high"]],
    // SiliconFlow .com 的 M3：平台级 enable_thinking 布尔开关（后端按平台
    // 推断兜底），M3 官方可关思考 → 两态
    ["SiliconFlow en", "MiniMaxAI/MiniMax-M3", ["none", "high"]],
  ];

  it.each(EXPECTED)(
    "%s / %s declares the vendor-documented levels",
    (presetName, modelId, levels) => {
      const model = catalogModel(presetName, modelId);
      expect(model.reasoningLevels).toEqual(levels);
      // 默认档一律不显式声明：后端 fallback（模板默认 high ∈ 声明子集时保留）
      // 在上述每一家都自然落到正确的 high
      expect(model.defaultReasoningLevel).toBeUndefined();
    },
  );

  it("keeps deliberately-unfilled presets unfilled", () => {
    // DeepSeek 直连走官方 catalog 镜像（已带 low/high/max），预填会覆盖官方声明；
    // MiniMax/MiMo 官方 catalog 就是 none/high 与模板默认一致，填了是零效果改动；
    // Bailian qwen3-coder-plus 无 per-model 档位证据
    const UNFILLED: Array<[string, string]> = [
      ["DeepSeek", "deepseek-v4-flash"],
      ["DeepSeek", "deepseek-v4-pro"],
      ["MiniMax", "MiniMax-M3"],
      ["MiniMax en", "MiniMax-M3"],
      ["Xiaomi MiMo", "mimo-v2.5-pro"],
      ["Xiaomi MiMo Token Plan (China)", "mimo-v2.5-pro"],
      ["Bailian", "qwen3-coder-plus"],
      // 千帆 Token Plan：三模型均不在 thinking 官方清单（2026-05-27 版）且
      // 无任何官方接入示例下发思考字段——无证据不造档位
      ["Baidu Qianfan Token Plan", "deepseek-v4-flash-0731"],
      ["Baidu Qianfan Token Plan", "glm-5.2"],
      ["Baidu Qianfan Token Plan", "kimi-k2.6"],
      // StepFun 无后缀 3.5-flash：官方未暴露 effort，单一常开思考态
      ["StepFun", "step-3.5-flash"],
      ["StepFun en", "step-3.5-flash"],
      // SiliconFlow .cn 的 M2.5 能否真正关思考无官方明文、ModelScope 是否
      // 透传思考字段未证实——真机验证前不造两态假开关（2026-08-15 盘点结论）
      ["SiliconFlow", "Pro/MiniMaxAI/MiniMax-M2.5"],
      ["ModelScope", "ZhipuAI/GLM-5.2"],
    ];
    for (const [presetName, modelId] of UNFILLED) {
      const model = catalogModel(presetName, modelId);
      expect(
        model.reasoningLevels,
        `${presetName}/${modelId} must stay unfilled`,
      ).toBeUndefined();
    }
  });

  it("only ever declares canonical Codex efforts", () => {
    for (const preset of codexProviderPresets) {
      for (const model of preset.modelCatalog ?? []) {
        for (const level of model.reasoningLevels ?? []) {
          expect(
            CANONICAL_EFFORTS,
            `${preset.name}/${model.model} level "${level}"`,
          ).toContain(level);
        }
        if (model.defaultReasoningLevel !== undefined) {
          expect(model.reasoningLevels ?? []).toContain(
            model.defaultReasoningLevel,
          );
        }
      }
    }
  });
});
