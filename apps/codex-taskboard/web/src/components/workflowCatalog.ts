import bytedanceLogo from "@lobehub/icons-static-svg/icons/bytedance-color.svg";
import claudeLogo from "@lobehub/icons-static-svg/icons/claude-color.svg";
import claudeCodeLogo from "@lobehub/icons-static-svg/icons/claudecode-color.svg";
import cloudflareLogo from "@lobehub/icons-static-svg/icons/cloudflare-color.svg";
import codexLogo from "@lobehub/icons-static-svg/icons/codex-color.svg";
import geminiLogo from "@lobehub/icons-static-svg/icons/gemini-color.svg";
import jimengLogo from "@lobehub/icons-static-svg/icons/jimeng-color.svg";
import klingLogo from "@lobehub/icons-static-svg/icons/kling-color.svg";
import mcpLogo from "@lobehub/icons-static-svg/icons/mcp.svg";
import midjourneyLogo from "@lobehub/icons-static-svg/icons/midjourney.svg";
import vercelLogo from "@lobehub/icons-static-svg/icons/vercel.svg";
import xLogo from "../assets/x-logo-black.png";
import type { WorkflowCapabilities } from "../types";
import type { WorkflowNodeData } from "./WorkflowNode";
export { WORKFLOW_TRIGGER_KINDS, isWorkflowTriggerKind } from "../../../shared/workflow-control-flow.mjs";

export type WorkflowGroup =
  | "触发器"
  | "流程控制"
  | "Skill 和 MCP"
  | "API"
  | "第三方集成"
  | "开发"
  | "规划"
  | "结果";

export interface PaletteItem {
  group: WorkflowGroup;
  title: string;
  description: string;
  data: WorkflowNodeData;
}

export const WORKFLOW_GROUPS: WorkflowGroup[] = [
  "触发器",
  "流程控制",
  "Skill 和 MCP",
  "API",
  "第三方集成",
  "开发",
  "规划",
  "结果",
];

export const GIT_OPERATIONS = [
  { value: "status", label: "查看状态" },
  { value: "commit", label: "提交更改" },
  { value: "pull", label: "拉取更新" },
  { value: "push", label: "推送分支" },
  { value: "create-branch", label: "创建分支" },
  { value: "switch-branch", label: "切换分支" },
  { value: "merge-branch", label: "合并分支" },
  { value: "create-worktree", label: "创建 Worktree" },
] as const;

export const ISSUE_STATUSES = [
  { value: "backlog", label: "积压事项" },
  { value: "todo", label: "待办事项" },
  { value: "in_progress", label: "进行中" },
  { value: "in_review", label: "审核中" },
  { value: "blocked", label: "已阻塞" },
  { value: "done", label: "完成" },
  { value: "canceled", label: "已取消" },
] as const;

export const ISSUE_PRIORITIES = [
  { value: "none", label: "无优先级" },
  { value: "urgent", label: "紧急" },
  { value: "high", label: "高" },
  { value: "medium", label: "中" },
  { value: "low", label: "低" },
] as const;

export const CONDITION_FIELDS = [
  {
    value: "issue-status",
    label: "任务状态",
    operators: ["equals", "not-equals"],
    defaultOperator: "equals",
    defaultValue: "todo",
  },
  {
    value: "issue-priority",
    label: "任务优先级",
    operators: ["equals", "not-equals"],
    defaultOperator: "equals",
    defaultValue: "none",
  },
  {
    value: "issue-labels",
    label: "任务标签",
    operators: ["contains", "not-contains"],
    defaultOperator: "contains",
    defaultValue: "",
  },
  {
    value: "upstream-output",
    label: "上游节点输出",
    operators: ["equals", "not-equals", "contains", "not-contains"],
    defaultOperator: "equals",
    defaultValue: "",
  },
] as const;

export const CONDITION_OPERATORS = [
  { value: "equals", label: "等于" },
  { value: "not-equals", label: "不等于" },
  { value: "contains", label: "包含" },
  { value: "not-contains", label: "不包含" },
] as const;

export const FEISHU_MESSAGE_RECIPIENTS = [
  { value: "self", label: "发送给自己" },
  { value: "user", label: "发送给特定用户" },
  { value: "chat", label: "发送到群聊" },
] as const;

export const CODE_RUNTIMES = [
  { value: "shell", label: "Shell" },
  { value: "javascript", label: "JavaScript" },
  { value: "python", label: "Python" },
] as const;

export const TEST_SCOPES = [
  { value: "related", label: "相关测试" },
  { value: "all", label: "全部测试" },
  { value: "custom", label: "自定义命令" },
] as const;

const FEISHU_LOGO = "https://p1-hera.feishucdn.com/tos-cn-i-jbbdkfciu3/84a9f036fe2b44f99b899fff4beeb963~tplv-jbbdkfciu3-image:0:0.image";
const GIT_LOGO = "https://git-scm.com/images/logos/downloads/Git-Icon-1788C.svg";

export const PALETTE_ITEMS: PaletteItem[] = [
  {
    group: "触发器",
    title: "Issue",
    description: "当任务变化时启动流程",
    data: {
      kind: "issue-trigger",
      eyebrow: "ISSUE TRIGGER",
      title: "任务触发器",
      description: "当任务满足条件时启动",
      meta: "状态、标签或优先级",
      icon: "myIssues",
      tone: "issue",
      outputLabel: "任务",
      triggerStatus: "todo",
    },
  },
  {
    group: "触发器",
    title: "RSS 订阅更新",
    description: "RSS 订阅发布新内容时启动流程",
    data: {
      kind: "rss-trigger",
      eyebrow: "RSS TRIGGER",
      title: "RSS 订阅更新",
      description: "指定的 RSS 订阅发布新内容时触发",
      meta: "尚未设置 RSS 订阅地址",
      icon: "recurrence",
      tone: "issue",
      outputLabel: "订阅条目",
      rssFeedUrl: "",
    },
  },
  {
    group: "触发器",
    title: "PR 提交",
    description: "当前项目仓库提交 PR 时启动流程",
    data: {
      kind: "pull-request-submitted-trigger",
      eyebrow: "PR TRIGGER",
      title: "PR 提交",
      description: "当前项目仓库提交新的 Pull Request 时触发",
      meta: "当前项目仓库 · Pull Request",
      icon: "branch",
      tone: "issue",
      outputLabel: "Pull Request",
    },
  },
  {
    group: "触发器",
    title: "Issue 提交",
    description: "当前项目仓库提交 Issue 时启动流程",
    data: {
      kind: "repository-issue-submitted-trigger",
      eyebrow: "ISSUE TRIGGER",
      title: "Issue 提交",
      description: "当前项目仓库提交新的 Issue 时触发",
      meta: "当前项目仓库 · Issue",
      icon: "createIssue",
      tone: "issue",
      outputLabel: "Issue",
    },
  },
  {
    group: "触发器",
    title: "Git 状态",
    description: "当前项目的 Git 工作区状态变化时启动流程",
    data: {
      kind: "git-status-trigger",
      eyebrow: "GIT TRIGGER",
      title: "Git 状态",
      description: "当前项目的 Git 工作区状态发生变化时触发",
      meta: "当前项目 · Git 工作区",
      icon: "branch",
      logo: GIT_LOGO,
      tone: "issue",
      outputLabel: "Git 状态",
    },
  },
  {
    group: "流程控制",
    title: "条件判断",
    description: "根据判断结果进入对应路径",
    data: {
      kind: "condition",
      eyebrow: "CONDITION",
      title: "条件判断",
      description: "根据判断结果进入对应路径",
      meta: "配置一个判断规则",
      icon: "filter",
      tone: "planning",
      inputLabel: "待判断数据",
      outputLabel: "符合条件的数据",
      conditionField: "issue-status",
      conditionOperator: "equals",
      conditionValue: "todo",
    },
  },
  {
    group: "Skill 和 MCP",
    title: "Skill",
    description: "调用已安装的 Skill",
    data: {
      kind: "skill",
      eyebrow: "SKILL",
      title: "调用 Skill",
      description: "运行工作区中的 Skill",
      meta: "选择一个 Skill",
      icon: "file",
      tone: "capability",
      inputLabel: "上下文",
      outputLabel: "输出",
    },
  },
  {
    group: "Skill 和 MCP",
    title: "MCP",
    description: "调用 MCP 工具或资源",
    data: {
      kind: "mcp",
      eyebrow: "MCP",
      title: "调用 MCP",
      description: "连接已配置的 MCP Server",
      meta: "选择一个 MCP Server",
      icon: "panel",
      logo: mcpLogo,
      logoMonochrome: true,
      tone: "capability",
      inputLabel: "参数",
      outputLabel: "结果",
    },
  },
  {
    group: "API",
    title: "Nano Banana 生图",
    description: "调用 Gemini 图像生成能力",
    data: {
      kind: "nano-banana",
      eyebrow: "IMAGE API",
      title: "Nano Banana 生图",
      description: "根据提示词和参考图生成图像",
      meta: "Google Gemini · Image",
      icon: "send",
      logo: geminiLogo,
      tone: "api",
      inputLabel: "提示词",
      outputLabel: "图像",
    },
  },
  {
    group: "API",
    title: "即梦生图",
    description: "调用即梦 AI 图片生成",
    data: {
      kind: "jimeng-image",
      eyebrow: "IMAGE API",
      title: "即梦生图",
      description: "使用即梦模型生成图片素材",
      meta: "即梦 AI · Image",
      icon: "send",
      logo: jimengLogo,
      tone: "api",
      inputLabel: "提示词",
      outputLabel: "图像",
    },
  },
  {
    group: "API",
    title: "Midjourney 生图",
    description: "提交 Midjourney 生成任务",
    data: {
      kind: "midjourney-image",
      eyebrow: "IMAGE API",
      title: "Midjourney 生图",
      description: "通过 Midjourney 生成图片素材",
      meta: "Midjourney · Image",
      icon: "send",
      logo: midjourneyLogo,
      logoMonochrome: true,
      tone: "api",
      inputLabel: "提示词",
      outputLabel: "图像",
    },
  },
  {
    group: "API",
    title: "Seedance 2.0 生视频",
    description: "调用字节跳动视频生成模型",
    data: {
      kind: "seedance-video",
      eyebrow: "VIDEO API",
      title: "Seedance 2.0 生视频",
      description: "生成多模态音视频内容",
      meta: "ByteDance Seed · Video",
      icon: "send",
      logo: bytedanceLogo,
      tone: "api",
      inputLabel: "素材与提示词",
      outputLabel: "视频",
    },
  },
  {
    group: "API",
    title: "可灵生视频",
    description: "调用可灵 AI 视频生成",
    data: {
      kind: "kling-video",
      eyebrow: "VIDEO API",
      title: "可灵生视频",
      description: "使用可灵模型生成视频素材",
      meta: "Kling AI · Video",
      icon: "send",
      logo: klingLogo,
      tone: "api",
      inputLabel: "素材与提示词",
      outputLabel: "视频",
    },
  },
  {
    group: "API",
    title: "自定义 API 节点",
    description: "配置任意 HTTP API",
    data: {
      kind: "custom-api",
      eyebrow: "HTTP API",
      title: "自定义 API 节点",
      description: "调用自定义 HTTP 接口",
      meta: "GET、POST、PUT",
      icon: "send",
      tone: "api",
      inputLabel: "请求",
      outputLabel: "响应",
    },
  },
  {
    group: "第三方集成",
    title: "Git",
    description: "读取仓库、分支与变更信息",
    data: {
      kind: "git",
      eyebrow: "INTEGRATION",
      title: "Git",
      description: "读取或操作当前项目的 Git 仓库",
      meta: "Git · Repository",
      icon: "branch",
      logo: GIT_LOGO,
      tone: "integration",
      inputLabel: "仓库与操作",
      outputLabel: "Git 结果",
      gitOperation: "commit",
      gitCommitMessage: "",
      gitStageAll: true,
      gitRemote: "origin",
      gitBranchName: "",
      gitBaseBranch: "",
      gitWorktreePath: "",
    },
  },
  {
    group: "第三方集成",
    title: "飞书文档",
    description: "读取或写入飞书云文档",
    data: {
      kind: "feishu-docs",
      eyebrow: "INTEGRATION",
      title: "飞书文档",
      description: "连接飞书文档与知识空间",
      meta: "飞书开放平台 · Docs",
      icon: "file",
      logo: FEISHU_LOGO,
      tone: "integration",
      inputLabel: "文档参数",
      outputLabel: "文档内容",
    },
  },
  {
    group: "第三方集成",
    title: "飞书消息",
    description: "发送消息给自己、用户或群聊",
    data: {
      kind: "feishu-message",
      eyebrow: "INTEGRATION",
      title: "飞书消息",
      description: "通过飞书开放平台发送消息",
      meta: "飞书开放平台 · IM",
      icon: "conversation",
      logo: FEISHU_LOGO,
      tone: "integration",
      inputLabel: "消息内容",
      outputLabel: "消息回执",
      feishuRecipientType: "self",
      feishuUserId: "",
      feishuChatId: "",
    },
  },
  {
    group: "第三方集成",
    title: "发布到 Twitter",
    description: "将内容发布到 Twitter",
    data: {
      kind: "twitter-post",
      eyebrow: "INTEGRATION",
      title: "发布到 Twitter",
      description: "将指定内容发布到 Twitter",
      meta: "尚未填写发布内容",
      icon: "send",
      logo: xLogo,
      logoMonochrome: true,
      tone: "integration",
      inputLabel: "发布内容",
      outputLabel: "发布结果",
      twitterPostContent: "",
    },
  },
  {
    group: "第三方集成",
    title: "OpenCLI",
    description: "调用网站适配器和登录态浏览器",
    data: {
      kind: "opencli",
      eyebrow: "INTEGRATION",
      title: "OpenCLI",
      description: "通过 OpenCLI 操作网站与本地工具",
      meta: "OpenCLI · Browser",
      icon: "panel",
      tone: "integration",
      inputLabel: "命令",
      outputLabel: "执行结果",
    },
  },
  {
    group: "第三方集成",
    title: "Claude Design 设计",
    description: "调用 Claude 生成设计方案",
    data: {
      kind: "claude-design",
      eyebrow: "INTEGRATION",
      title: "Claude Design 设计",
      description: "使用 Claude 完成设计与实现",
      meta: "Claude · Design",
      icon: "write",
      logo: claudeLogo,
      tone: "integration",
      inputLabel: "设计需求",
      outputLabel: "设计结果",
    },
  },
  {
    group: "第三方集成",
    title: "Cloudflare 部署",
    description: "部署 Workers、Pages 等服务",
    data: {
      kind: "cloudflare-deploy",
      eyebrow: "DEPLOYMENT",
      title: "Cloudflare 部署",
      description: "构建并部署到 Cloudflare",
      meta: "Workers · Pages",
      icon: "send",
      logo: cloudflareLogo,
      tone: "integration",
      inputLabel: "构建产物",
      outputLabel: "部署地址",
    },
  },
  {
    group: "第三方集成",
    title: "Vercel 部署",
    description: "部署项目并返回预览地址",
    data: {
      kind: "vercel-deploy",
      eyebrow: "DEPLOYMENT",
      title: "Vercel 部署",
      description: "构建并部署到 Vercel",
      meta: "Preview · Production",
      icon: "send",
      logo: vercelLogo,
      logoMonochrome: true,
      tone: "integration",
      inputLabel: "构建产物",
      outputLabel: "部署地址",
    },
  },
  {
    group: "第三方集成",
    title: "自定义集成",
    description: "连接其他第三方服务",
    data: {
      kind: "custom-integration",
      eyebrow: "INTEGRATION",
      title: "自定义集成",
      description: "通过授权或 Webhook 连接服务",
      meta: "OAuth · Webhook",
      icon: "link",
      tone: "integration",
      inputLabel: "集成参数",
      outputLabel: "执行结果",
    },
  },
  {
    group: "开发",
    title: "自定义代码",
    description: "使用自定义脚本处理流程数据",
    data: {
      kind: "custom-code",
      eyebrow: "CODE",
      title: "自定义代码",
      description: "在当前项目上下文中运行自定义代码",
      meta: "运行环境 · Shell",
      icon: "panel",
      tone: "development",
      inputLabel: "流程数据",
      outputLabel: "代码输出",
      codeRuntime: "shell",
      codeContent: "",
    },
  },
  {
    group: "开发",
    title: "写测试",
    description: "根据当前任务和项目上下文编写测试",
    data: {
      kind: "write-tests",
      eyebrow: "TEST",
      title: "写测试",
      description: "根据当前任务和项目上下文编写测试",
      meta: "当前项目 · 测试",
      icon: "write",
      tone: "development",
      inputLabel: "任务上下文",
      outputLabel: "测试代码",
    },
  },
  {
    group: "开发",
    title: "运行测试",
    description: "运行相关测试、全部测试或自定义命令",
    data: {
      kind: "run-tests",
      eyebrow: "TEST",
      title: "运行测试",
      description: "在当前项目中运行测试",
      meta: "测试范围 · 相关测试",
      icon: "check",
      tone: "development",
      inputLabel: "项目变更",
      outputLabel: "测试结果",
      testScope: "related",
      testCommand: "",
    },
  },
  {
    group: "规划",
    title: "基础规划",
    description: "拆解步骤、依赖和验收条件",
    data: {
      kind: "basic-planning",
      eyebrow: "PLANNING",
      title: "基础规划",
      description: "根据任务生成结构化执行计划",
      meta: "内置规划器",
      icon: "dashboard",
      tone: "planning",
      inputLabel: "任务上下文",
      outputLabel: "执行计划",
      acceptsChildren: true,
    },
  },
  {
    group: "规划",
    title: "Claude Code 规划",
    description: "使用 Claude Code 生成计划",
    data: {
      kind: "claude-code-planning",
      eyebrow: "PLANNING",
      title: "Claude Code 规划",
      description: "让 Claude Code 分析并规划任务",
      meta: "Claude Code · Plan",
      icon: "dashboard",
      logo: claudeCodeLogo,
      tone: "planning",
      inputLabel: "任务上下文",
      outputLabel: "执行计划",
      claudeModel: "claude-sonnet",
      reasoningEffort: "high",
      planningRequirements: "分析依赖、风险、执行步骤和验收条件，输出可直接执行的计划。",
    },
  },
  {
    group: "规划",
    title: "自定义规划",
    description: "通过自定义提示词生成计划",
    data: {
      kind: "custom-planning",
      eyebrow: "PLANNING",
      title: "自定义规划",
      description: "使用自定义规则拆解任务",
      meta: "Prompt · 自定义",
      icon: "write",
      tone: "planning",
      inputLabel: "任务上下文",
      outputLabel: "执行计划",
    },
  },
  {
    group: "结果",
    title: "添加 ISSUE",
    description: "在当前项目中创建新任务",
    data: {
      kind: "issue-create",
      eyebrow: "ISSUE ACTION",
      title: "添加 ISSUE",
      description: "在当前流程所属项目中创建任务",
      meta: "待填写任务标题",
      icon: "createIssue",
      tone: "result",
      inputLabel: "流程上下文",
      outputLabel: "新任务",
      createIssueTitle: "",
      createIssueDescription: "",
      createIssueStatus: "todo",
      createIssuePriority: "none",
      createIssueLabels: "",
    },
  },
  {
    group: "结果",
    title: "更新 Issue",
    description: "回写状态、评论和附件",
    data: {
      kind: "issue-update",
      eyebrow: "ISSUE ACTION",
      title: "更新任务",
      description: "把流程结果写回任务",
      meta: "状态、评论或附件",
      icon: "write",
      tone: "result",
      inputLabel: "流程结果",
      outputLabel: "已更新",
      issueTarget: "trigger",
      specificIssueId: "",
      changeStatus: true,
      targetStatus: "in_review",
      addComment: true,
      commentSource: "workflow-output",
      customComment: "",
      addLabels: false,
      labelsToAdd: "",
      setPriority: false,
      targetPriority: "none",
      attachArtifacts: true,
      recordConversation: true,
    },
  },
  {
    group: "结果",
    title: "Codex 审核",
    description: "由 Codex 审核结果和变更",
    data: {
      kind: "codex-review",
      eyebrow: "REVIEW",
      title: "Codex 审核",
      description: "检查实现结果、测试与验收条件",
      meta: "Codex · Review",
      icon: "check",
      logo: codexLogo,
      tone: "result",
      inputLabel: "执行结果",
      outputLabel: "审核结论",
    },
  },
  {
    group: "结果",
    title: "Claude Code 审核",
    description: "由 Claude Code 审核结果和变更",
    data: {
      kind: "claude-code-review",
      eyebrow: "REVIEW",
      title: "Claude Code 审核",
      description: "使用 Claude Code 复核实现结果",
      meta: "Claude Code · Review",
      icon: "check",
      logo: claudeCodeLogo,
      tone: "result",
      inputLabel: "执行结果",
      outputLabel: "审核结论",
      claudeModel: "claude-sonnet",
      reasoningEffort: "high",
      planningRequirements: "对照执行计划和验收条件复核变更、测试结果与潜在回归。",
    },
  },
];

export function paletteData(kind: string): WorkflowNodeData {
  return PALETTE_ITEMS.find((item) => item.data.kind === kind)!.data;
}

export function optionLabel(
  options: readonly { value: string; label: string }[],
  value: string | undefined,
): string {
  return options.find((option) => option.value === value)?.label ?? "";
}

export function selectedCapabilityValue(
  options: readonly { id: string }[],
  value: string | undefined,
): string {
  return value && options.some((option) => option.id === value) ? value : "";
}

export function capabilityNodeMeta(
  data: WorkflowNodeData,
  capabilities: WorkflowCapabilities | null,
  failed: boolean,
): string {
  if (data.kind === "issue-create") {
    const status = optionLabel(ISSUE_STATUSES, data.createIssueStatus ?? "todo");
    const priority = optionLabel(ISSUE_PRIORITIES, data.createIssuePriority ?? "none");
    return `初始状态 · ${status} · 优先级 ${priority}`;
  }
  if (data.kind === "rss-trigger") {
    const source = rssSourceLabel(data.rssFeedUrl);
    return source ? `RSS · ${source}` : "尚未设置 RSS 订阅地址";
  }
  if (data.kind === "twitter-post") {
    const content = data.twitterPostContent?.trim();
    return content ? `发布内容 · ${twitterPostSummary(content)}` : "尚未填写发布内容";
  }
  if (data.kind === "custom-code") {
    return `运行环境 · ${optionLabel(CODE_RUNTIMES, data.codeRuntime ?? "shell")}`;
  }
  if (data.kind === "run-tests") {
    return `测试范围 · ${optionLabel(TEST_SCOPES, data.testScope ?? "related")}`;
  }
  if (data.kind === "skill") {
    if (!capabilities) return "正在读取可用 Skill";
    if (failed) return "无法读取可用 Skill";
    const skill = capabilities.skills.find((option) => option.id === data.selectedSkill);
    if (skill) return `${skill.label} · Skill`;
    return data.selectedSkill ? "所选 Skill 当前不可用" : "尚未选择 Skill";
  }
  if (data.kind === "mcp") {
    if (!capabilities) return "正在读取可用 MCP Server";
    if (failed) return "无法读取可用 MCP Server";
    const server = capabilities.mcpServers.find((option) => option.id === data.selectedMcpServer);
    if (server) return `${server.label} · ${server.transport}`;
    return data.selectedMcpServer ? "所选 MCP Server 当前不可用" : "尚未选择 MCP Server";
  }
  return data.meta;
}

function formatActionTitle(title: string, actions: string[]): string {
  if (actions.length === 0) return title;
  const visibleActions = actions.slice(0, 2).join("、");
  const remaining = actions.length > 2 ? ` +${actions.length - 2}` : "";
  return `${title} · ${visibleActions}${remaining}`;
}

function rssSourceLabel(value: string | undefined): string {
  const feedUrl = value?.trim();
  if (!feedUrl) return "";
  return feedUrl.replace(/^https?:\/\//i, "").split(/[/?#]/)[0] || feedUrl;
}

function twitterPostSummary(value: string): string {
  const normalized = value.replace(/\s+/g, " ").trim();
  return normalized.length > 36 ? `${normalized.slice(0, 36)}…` : normalized;
}

export function workflowNodeDisplayTitle(data: WorkflowNodeData): string {
  if (data.kind === "issue-create") {
    const issueTitle = data.createIssueTitle?.trim();
    return formatActionTitle(data.title, issueTitle ? [issueTitle] : []);
  }
  if (data.kind === "rss-trigger") {
    const source = rssSourceLabel(data.rssFeedUrl);
    return formatActionTitle(data.title, source ? [source] : []);
  }
  if (data.kind === "twitter-post") {
    const content = data.twitterPostContent?.trim();
    return formatActionTitle(data.title, content ? [twitterPostSummary(content)] : []);
  }
  if (data.kind === "condition") {
    const field = CONDITION_FIELDS.find(
      (option) => option.value === data.conditionField,
    ) ?? CONDITION_FIELDS[0];
    const operatorValue = field.operators.find(
      (value) => value === data.conditionOperator,
    ) ?? field.defaultOperator;
    const value = data.conditionValue || field.defaultValue;
    const valueLabel = field.value === "issue-status"
      ? optionLabel(ISSUE_STATUSES, value)
      : field.value === "issue-priority"
        ? optionLabel(ISSUE_PRIORITIES, value)
        : value;
    return formatActionTitle(data.title, [
      `${field.label} ${optionLabel(CONDITION_OPERATORS, operatorValue)} ${valueLabel || "未设置"}`,
    ]);
  }
  if (data.kind === "feishu-message") {
    return formatActionTitle(data.title, [
      optionLabel(FEISHU_MESSAGE_RECIPIENTS, data.feishuRecipientType ?? "self"),
    ]);
  }
  if (data.kind === "git") {
    return formatActionTitle(data.title, [
      optionLabel(GIT_OPERATIONS, data.gitOperation ?? "commit"),
    ]);
  }
  if (data.kind === "custom-code") {
    return formatActionTitle(data.title, [
      optionLabel(CODE_RUNTIMES, data.codeRuntime ?? "shell"),
    ]);
  }
  if (data.kind === "run-tests") {
    return formatActionTitle(data.title, [
      optionLabel(TEST_SCOPES, data.testScope ?? "related"),
    ]);
  }
  if (data.kind === "issue-trigger") {
    const status = optionLabel(ISSUE_STATUSES, data.triggerStatus ?? "todo");
    return formatActionTitle(data.title, [`进入${status}`]);
  }
  if (data.kind === "issue-update") {
    const actions = [
      data.changeStatus
        ? `状态 → ${optionLabel(ISSUE_STATUSES, data.targetStatus ?? "in_review")}`
        : "",
      data.addComment ? "添加评论" : "",
      data.addLabels ? "添加标签" : "",
      data.setPriority
        ? `优先级 → ${optionLabel(ISSUE_PRIORITIES, data.targetPriority ?? "none")}`
        : "",
      data.attachArtifacts ? "附加产物" : "",
      data.recordConversation ? "记录对话" : "",
    ].filter(Boolean);
    return formatActionTitle(data.title, actions);
  }
  return data.title;
}

export function workflowNodeConfigured(
  data: WorkflowNodeData,
  capabilities: WorkflowCapabilities | null,
  failed: boolean,
): boolean {
  if (data.kind === "issue-create") {
    return Boolean(data.createIssueTitle?.trim());
  }
  if (data.kind === "rss-trigger") {
    return Boolean(data.rssFeedUrl?.trim());
  }
  if (data.kind === "twitter-post") {
    return Boolean(data.twitterPostContent?.trim());
  }
  if (data.kind === "condition") {
    const field = CONDITION_FIELDS.find(
      (option) => option.value === data.conditionField,
    ) ?? CONDITION_FIELDS[0];
    if (!field.operators.some((operator) => operator === data.conditionOperator)) return false;
    if (field.value === "issue-status") {
      return ISSUE_STATUSES.some((status) => status.value === data.conditionValue);
    }
    if (field.value === "issue-priority") {
      return ISSUE_PRIORITIES.some((priority) => priority.value === data.conditionValue);
    }
    return Boolean(data.conditionValue?.trim());
  }
  if (data.kind === "feishu-message") {
    if (data.feishuRecipientType === "user") return Boolean(data.feishuUserId?.trim());
    if (data.feishuRecipientType === "chat") return Boolean(data.feishuChatId?.trim());
    return true;
  }
  if (data.kind === "custom-code") {
    return Boolean(data.codeContent?.trim());
  }
  if (data.kind === "run-tests") {
    if (data.testScope === "custom") return Boolean(data.testCommand?.trim());
    return true;
  }
  if (data.kind === "skill") {
    return !failed
      && Boolean(data.selectedSkill)
      && Boolean(capabilities?.skills.some((item) => item.id === data.selectedSkill));
  }
  if (data.kind === "mcp") {
    return !failed
      && Boolean(data.selectedMcpServer)
      && Boolean(capabilities?.mcpServers.some((item) => item.id === data.selectedMcpServer));
  }
  if (data.kind === "issue-update") {
    return Boolean(
      data.changeStatus
      || data.addComment
      || data.addLabels
      || data.setPriority
      || data.attachArtifacts
      || data.recordConversation,
    );
  }
  return true;
}
