import { useState } from "react";
import type { WorkflowCapabilities } from "../types";
import { LinearIcon } from "./LinearIcon";
import {
  type WorkflowCanvasNode,
  type WorkflowNodeData,
} from "./WorkflowNode";
import {
  CODE_RUNTIMES,
  CONDITION_FIELDS,
  CONDITION_OPERATORS,
  FEISHU_MESSAGE_RECIPIENTS,
  GIT_OPERATIONS,
  ISSUE_PRIORITIES,
  ISSUE_STATUSES,
  TEST_SCOPES,
  selectedCapabilityValue,
  workflowNodeDisplayTitle,
} from "./workflowCatalog";
import { WorkflowMark } from "./WorkflowMark";

interface WorkflowInspectorProps {
  node: WorkflowCanvasNode;
  projectName: string;
  capabilities: WorkflowCapabilities | null;
  capabilitiesFailed: boolean;
  onChange: (changes: Partial<WorkflowNodeData>) => void;
  onClose: () => void;
}

type InspectorTab = "settings" | "configuration";

export function WorkflowInspector({
  node,
  projectName,
  capabilities,
  capabilitiesFailed,
  onChange,
  onClose,
}: WorkflowInspectorProps) {
  const [activeTab, setActiveTab] = useState<InspectorTab>("settings");
  const data = node.data;
  const conditionField = data.conditionField ?? CONDITION_FIELDS[0].value;
  const selectedConditionField = CONDITION_FIELDS.find(
    (field) => field.value === conditionField,
  ) ?? CONDITION_FIELDS[0];
  const conditionOperator = selectedConditionField.operators.find(
    (operator) => operator === data.conditionOperator,
  ) ?? selectedConditionField.defaultOperator;
  const conditionValue = data.conditionValue || selectedConditionField.defaultValue;

  return (
    <div className="workflow-inspector-content">
      <div className={`workflow-inspector-title workflow-inspector-${data.tone}`}>
        <span aria-hidden="true">
          <WorkflowMark
            icon={data.icon}
            logo={data.logo}
            logoMonochrome={data.logoMonochrome}
          />
        </span>
        <div>
          <small>{data.eyebrow}</small>
          <strong>{workflowNodeDisplayTitle(data)}</strong>
        </div>
        <button
          className="workflow-panel-toggle"
          type="button"
          aria-label="关闭步骤配置"
          title="关闭步骤配置"
          onClick={onClose}
        >
          <LinearIcon name="close" />
        </button>
      </div>

      <div className="workflow-inspector-tabs" role="tablist" aria-label="步骤配置视图">
        <button
          className={activeTab === "settings" ? "is-active" : ""}
          type="button"
          role="tab"
          aria-selected={activeTab === "settings"}
          onClick={() => setActiveTab("settings")}
        >设置</button>
        <button
          className={activeTab === "configuration" ? "is-active" : ""}
          type="button"
          role="tab"
          aria-selected={activeTab === "configuration"}
          onClick={() => setActiveTab("configuration")}
        >配置</button>
      </div>

      {activeTab === "settings" ? (
        <div role="tabpanel" aria-label="设置">
          <div className="workflow-config-section">
            <h2>常规</h2>
            <label>
              <span>节点名称</span>
              <input
                type="text"
                value={data.title}
                onChange={(event) => onChange({ title: event.target.value })}
              />
            </label>
            <label>
              <span>说明</span>
              <textarea
                rows={3}
                value={data.description}
                onChange={(event) => onChange({ description: event.target.value })}
              />
            </label>
          </div>

          <div className="workflow-config-section">
            <h2>额外说明</h2>
            <textarea
              aria-label="额外说明"
              rows={4}
              value={data.additionalInstructions ?? ""}
              placeholder="补充执行约束、上下文或验收要求…"
              onChange={(event) => onChange({ additionalInstructions: event.target.value })}
            />
          </div>

          <div className="workflow-config-section">
            <h2>上下文</h2>
            <div className="workflow-context-field">
              <span>
                <LinearIcon name="project" />
                当前项目
              </span>
              <strong>{projectName}</strong>
              <LinearIcon name="chevronDown" />
            </div>
          </div>
        </div>
      ) : (
        <div role="tabpanel" aria-label="配置">
          {data.kind === "issue-create" && (
            <div className="workflow-config-section">
              <h2>创建任务</h2>
              <label>
                <span>标题</span>
                <input
                  aria-label="ISSUE 标题"
                  type="text"
                  value={data.createIssueTitle ?? ""}
                  placeholder="输入任务标题"
                  onChange={(event) => onChange({ createIssueTitle: event.target.value })}
                />
              </label>
              <label>
                <span>描述</span>
                <textarea
                  aria-label="ISSUE 描述"
                  rows={4}
                  value={data.createIssueDescription ?? ""}
                  placeholder="补充任务描述…"
                  onChange={(event) => onChange({ createIssueDescription: event.target.value })}
                />
              </label>
              <label>
                <span>初始状态</span>
                <select
                  aria-label="ISSUE 初始状态"
                  value={data.createIssueStatus ?? "todo"}
                  onChange={(event) => onChange({ createIssueStatus: event.target.value })}
                >
                  {ISSUE_STATUSES.map((status) => (
                    <option key={status.value} value={status.value}>{status.label}</option>
                  ))}
                </select>
              </label>
              <label>
                <span>优先级</span>
                <select
                  aria-label="ISSUE 优先级"
                  value={data.createIssuePriority ?? "none"}
                  onChange={(event) => onChange({ createIssuePriority: event.target.value })}
                >
                  {ISSUE_PRIORITIES.map((priority) => (
                    <option key={priority.value} value={priority.value}>{priority.label}</option>
                  ))}
                </select>
              </label>
              <label>
                <span>标签</span>
                <input
                  aria-label="ISSUE 标签"
                  type="text"
                  value={data.createIssueLabels ?? ""}
                  placeholder="多个标签用逗号分隔"
                  onChange={(event) => onChange({ createIssueLabels: event.target.value })}
                />
              </label>
            </div>
          )}

          {data.kind === "skill" && (
            <div className="workflow-config-section">
              <h2>Skill</h2>
              <label>
                <span>可用 Skill</span>
                <select
                  aria-label="可用 Skill"
                  value={selectedCapabilityValue(
                    capabilities?.skills ?? [],
                    data.selectedSkill,
                  )}
                  disabled={
                    !capabilities
                    || capabilitiesFailed
                    || capabilities.skills.length === 0
                  }
                  onChange={(event) => onChange({
                    selectedSkill: event.target.value,
                    meta: `${event.target.selectedOptions[0].text} · Skill`,
                  })}
                >
                  <option value="" disabled>
                    {!capabilities
                      ? "正在读取可用 Skill…"
                      : capabilitiesFailed
                        ? "读取可用 Skill 失败"
                        : capabilities.skills.length === 0
                          ? "未发现可用 Skill"
                          : "请选择 Skill"}
                  </option>
                  {(capabilities?.skills ?? []).map((skill) => (
                    <option key={skill.id} value={skill.id}>{skill.label}</option>
                  ))}
                </select>
              </label>
            </div>
          )}

          {data.kind === "mcp" && (
            <div className="workflow-config-section">
              <h2>MCP</h2>
              <label>
                <span>可用 MCP Server</span>
                <select
                  aria-label="可用 MCP Server"
                  value={selectedCapabilityValue(
                    capabilities?.mcpServers ?? [],
                    data.selectedMcpServer,
                  )}
                  disabled={
                    !capabilities
                    || capabilitiesFailed
                    || capabilities.mcpServers.length === 0
                  }
                  onChange={(event) => {
                    const server = capabilities?.mcpServers.find(
                      (option) => option.id === event.target.value,
                    );
                    onChange({
                      selectedMcpServer: event.target.value,
                      meta: server
                        ? `${server.label} · ${server.transport}`
                        : "尚未选择 MCP Server",
                    });
                  }}
                >
                  <option value="" disabled>
                    {!capabilities
                      ? "正在读取可用 MCP Server…"
                      : capabilitiesFailed
                        ? "读取可用 MCP Server 失败"
                        : capabilities.mcpServers.length === 0
                          ? "未发现可用 MCP Server"
                          : "请选择 MCP Server"}
                  </option>
                  {(capabilities?.mcpServers ?? []).map((server) => (
                    <option key={server.id} value={server.id}>
                      {server.label} · {server.transport}
                    </option>
                  ))}
                </select>
              </label>
            </div>
          )}

          {data.kind === "rss-trigger" && (
            <div className="workflow-config-section">
              <h2>RSS 订阅</h2>
              <label>
                <span>订阅地址</span>
                <input
                  aria-label="RSS 订阅地址"
                  type="url"
                  value={data.rssFeedUrl ?? ""}
                  placeholder="https://example.com/feed.xml"
                  onChange={(event) => onChange({ rssFeedUrl: event.target.value })}
                />
              </label>
            </div>
          )}

          {data.kind === "condition" && (
            <div className="workflow-config-section">
              <h2>判断规则</h2>
              <label>
                <span>判断字段</span>
                <select
                  aria-label="判断字段"
                  value={conditionField}
                  onChange={(event) => {
                    const selectedField = CONDITION_FIELDS.find(
                      (field) => field.value === event.target.value,
                    )!;
                    onChange({
                      conditionField: selectedField.value,
                      conditionOperator: selectedField.defaultOperator,
                      conditionValue: selectedField.defaultValue,
                    });
                  }}
                >
                  {CONDITION_FIELDS.map((field) => (
                    <option key={field.value} value={field.value}>{field.label}</option>
                  ))}
                </select>
              </label>
              <label>
                <span>运算符</span>
                <select
                  aria-label="运算符"
                  value={conditionOperator}
                  onChange={(event) => onChange({ conditionOperator: event.target.value })}
                >
                  {selectedConditionField.operators.map((operatorValue) => {
                    const operator = CONDITION_OPERATORS.find(
                      (option) => option.value === operatorValue,
                    )!;
                    return (
                      <option key={operator.value} value={operator.value}>{operator.label}</option>
                    );
                  })}
                </select>
              </label>
              {conditionField === "issue-status" && (
                <label>
                  <span>比较值</span>
                  <select
                    aria-label="比较值"
                    value={conditionValue}
                    onChange={(event) => onChange({ conditionValue: event.target.value })}
                  >
                    {ISSUE_STATUSES.map((status) => (
                      <option key={status.value} value={status.value}>{status.label}</option>
                    ))}
                  </select>
                </label>
              )}
              {conditionField === "issue-priority" && (
                <label>
                  <span>比较值</span>
                  <select
                    aria-label="比较值"
                    value={conditionValue}
                    onChange={(event) => onChange({ conditionValue: event.target.value })}
                  >
                    {ISSUE_PRIORITIES.map((priority) => (
                      <option key={priority.value} value={priority.value}>{priority.label}</option>
                    ))}
                  </select>
                </label>
              )}
              {(conditionField === "issue-labels" || conditionField === "upstream-output") && (
                <label>
                  <span>比较值</span>
                  <input
                    aria-label="比较值"
                    type="text"
                    value={conditionValue}
                    placeholder="输入要比较的值"
                    onChange={(event) => onChange({ conditionValue: event.target.value })}
                  />
                </label>
              )}
            </div>
          )}

          {data.kind === "feishu-message" && (
            <div className="workflow-config-section">
              <h2>飞书消息</h2>
              <label>
                <span>发送对象</span>
                <select
                  aria-label="飞书消息发送对象"
                  value={data.feishuRecipientType ?? "self"}
                  onChange={(event) => onChange({
                    feishuRecipientType: event.target.value as WorkflowNodeData["feishuRecipientType"],
                    feishuUserId: "",
                    feishuChatId: "",
                  })}
                >
                  {FEISHU_MESSAGE_RECIPIENTS.map((recipient) => (
                    <option key={recipient.value} value={recipient.value}>{recipient.label}</option>
                  ))}
                </select>
              </label>
              {data.feishuRecipientType === "user" && (
                <label>
                  <span>用户 ID</span>
                  <input
                    aria-label="飞书用户"
                    type="text"
                    value={data.feishuUserId ?? ""}
                    placeholder="open_id 或 user_id"
                    onChange={(event) => onChange({ feishuUserId: event.target.value })}
                  />
                </label>
              )}
              {data.feishuRecipientType === "chat" && (
                <label>
                  <span>群聊 ID</span>
                  <input
                    aria-label="飞书群聊"
                    type="text"
                    value={data.feishuChatId ?? ""}
                    placeholder="chat_id"
                    onChange={(event) => onChange({ feishuChatId: event.target.value })}
                  />
                </label>
              )}
            </div>
          )}

          {data.kind === "twitter-post" && (
            <div className="workflow-config-section">
              <h2>发布到 Twitter</h2>
              <label>
                <span>发布内容</span>
                <textarea
                  aria-label="Twitter 发布内容"
                  rows={6}
                  value={data.twitterPostContent ?? ""}
                  placeholder="输入要发布的内容…"
                  onChange={(event) => onChange({ twitterPostContent: event.target.value })}
                />
              </label>
            </div>
          )}

          {data.kind === "git" && (
            <div className="workflow-config-section">
              <h2>Git 操作</h2>
              <label>
                <span>操作</span>
                <select
                  aria-label="Git 操作"
                  value={data.gitOperation ?? "commit"}
                  onChange={(event) => onChange({ gitOperation: event.target.value })}
                >
                  {GIT_OPERATIONS.map((operation) => (
                    <option key={operation.value} value={operation.value}>{operation.label}</option>
                  ))}
                </select>
              </label>
              {data.gitOperation === "commit" && (
                <>
                  <label>
                    <span>提交说明</span>
                    <input
                      aria-label="Git 提交说明"
                      type="text"
                      value={data.gitCommitMessage ?? ""}
                      placeholder="描述本次变更"
                      onChange={(event) => onChange({ gitCommitMessage: event.target.value })}
                    />
                  </label>
                  <label className="workflow-action-toggle workflow-action-toggle-full">
                    <input
                      type="checkbox"
                      checked={data.gitStageAll ?? true}
                      onChange={(event) => onChange({ gitStageAll: event.target.checked })}
                    />
                    <span>提交前暂存全部变更</span>
                  </label>
                </>
              )}
              {(data.gitOperation === "pull" || data.gitOperation === "push") && (
                <>
                  <label>
                    <span>远程仓库</span>
                    <input
                      aria-label="Git 远程仓库"
                      type="text"
                      value={data.gitRemote ?? "origin"}
                      placeholder="origin"
                      onChange={(event) => onChange({ gitRemote: event.target.value })}
                    />
                  </label>
                  <label>
                    <span>分支</span>
                    <input
                      aria-label="Git 分支"
                      type="text"
                      value={data.gitBranchName ?? ""}
                      placeholder="留空使用当前分支"
                      onChange={(event) => onChange({ gitBranchName: event.target.value })}
                    />
                  </label>
                </>
              )}
              {(data.gitOperation === "create-branch"
                || data.gitOperation === "switch-branch"
                || data.gitOperation === "merge-branch") && (
                <label>
                  <span>分支名称</span>
                  <input
                    aria-label="Git 分支名称"
                    type="text"
                    value={data.gitBranchName ?? ""}
                    placeholder="feature/workflow"
                    onChange={(event) => onChange({ gitBranchName: event.target.value })}
                  />
                </label>
              )}
              {(data.gitOperation === "create-branch"
                || data.gitOperation === "create-worktree") && (
                <label>
                  <span>基于分支</span>
                  <input
                    aria-label="Git 基于分支"
                    type="text"
                    value={data.gitBaseBranch ?? ""}
                    placeholder="留空使用当前分支"
                    onChange={(event) => onChange({ gitBaseBranch: event.target.value })}
                  />
                </label>
              )}
              {data.gitOperation === "create-worktree" && (
                <>
                  <label>
                    <span>Worktree 分支</span>
                    <input
                      aria-label="Git Worktree 分支"
                      type="text"
                      value={data.gitBranchName ?? ""}
                      placeholder="feature/workflow"
                      onChange={(event) => onChange({ gitBranchName: event.target.value })}
                    />
                  </label>
                  <label>
                    <span>Worktree 目录</span>
                    <input
                      aria-label="Git Worktree 目录"
                      type="text"
                      value={data.gitWorktreePath ?? ""}
                      placeholder="../project-worktree"
                      onChange={(event) => onChange({ gitWorktreePath: event.target.value })}
                    />
                  </label>
                </>
              )}
            </div>
          )}

          {data.kind === "custom-code" && (
            <div className="workflow-config-section">
              <h2>自定义代码</h2>
              <label>
                <span>运行环境</span>
                <select
                  aria-label="代码运行环境"
                  value={data.codeRuntime ?? "shell"}
                  onChange={(event) => onChange({ codeRuntime: event.target.value as WorkflowNodeData["codeRuntime"] })}
                >
                  {CODE_RUNTIMES.map((runtime) => (
                    <option key={runtime.value} value={runtime.value}>{runtime.label}</option>
                  ))}
                </select>
              </label>
              <label>
                <span>代码内容</span>
                <textarea
                  aria-label="代码内容"
                  rows={10}
                  value={data.codeContent ?? ""}
                  placeholder="输入要运行的代码…"
                  onChange={(event) => onChange({ codeContent: event.target.value })}
                />
              </label>
            </div>
          )}

          {data.kind === "run-tests" && (
            <div className="workflow-config-section">
              <h2>运行测试</h2>
              <label>
                <span>测试范围</span>
                <select
                  aria-label="测试范围"
                  value={data.testScope ?? "related"}
                  onChange={(event) => onChange({ testScope: event.target.value as WorkflowNodeData["testScope"] })}
                >
                  {TEST_SCOPES.map((scope) => (
                    <option key={scope.value} value={scope.value}>{scope.label}</option>
                  ))}
                </select>
              </label>
              {data.testScope === "custom" && (
                <label>
                  <span>测试命令</span>
                  <input
                    aria-label="测试命令"
                    type="text"
                    value={data.testCommand ?? ""}
                    placeholder="例如 npm test -- workflow"
                    onChange={(event) => onChange({ testCommand: event.target.value })}
                  />
                </label>
              )}
            </div>
          )}

          {(data.kind === "claude-code-planning" || data.kind === "claude-code-review") && (
            <div className="workflow-config-section">
              <h2>Claude Code</h2>
              <label>
                <span>模型</span>
                <select
                  aria-label="Claude Code 模型"
                  value={data.claudeModel ?? "claude-sonnet"}
                  onChange={(event) => onChange({ claudeModel: event.target.value })}
                >
                  <option value="claude-sonnet">Claude Sonnet</option>
                  <option value="claude-opus">Claude Opus</option>
                  <option value="claude-haiku">Claude Haiku</option>
                </select>
              </label>
              <label>
                <span>推理强度</span>
                <select
                  aria-label="推理强度"
                  value={data.reasoningEffort ?? "high"}
                  onChange={(event) => onChange({ reasoningEffort: event.target.value })}
                >
                  <option value="low">低</option>
                  <option value="medium">中</option>
                  <option value="high">高</option>
                  <option value="max">最高</option>
                </select>
              </label>
              <label>
                <span>规划要求</span>
                <textarea
                  rows={4}
                  value={data.planningRequirements ?? ""}
                  placeholder="说明分析步骤、约束、风险和验收要求…"
                  onChange={(event) => onChange({ planningRequirements: event.target.value })}
                />
              </label>
            </div>
          )}

          {data.kind === "issue-trigger" && (
            <div className="workflow-config-section">
              <h2>触发条件</h2>
              <label>
                <span>任务状态变为</span>
                <select
                  aria-label="任务触发状态"
                  value={data.triggerStatus ?? "todo"}
                  onChange={(event) => onChange({
                    triggerStatus: event.target.value,
                    description: `状态变为「${event.target.selectedOptions[0].text}」时触发`,
                  })}
                >
                  {ISSUE_STATUSES.map((status) => (
                    <option key={status.value} value={status.value}>{status.label}</option>
                  ))}
                </select>
              </label>
            </div>
          )}

          {data.kind === "issue-update" && (
            <div className="workflow-config-section">
              <h2>任务操作</h2>
              <label>
                <span>任务选择</span>
                <select
                  aria-label="任务选择"
                  value={data.issueTarget ?? "trigger"}
                  onChange={(event) => onChange({ issueTarget: event.target.value })}
                >
                  <option value="trigger">触发流程的任务</option>
                  <option value="upstream">上游节点输出的任务</option>
                  <option value="specific">指定任务</option>
                </select>
              </label>
              {data.issueTarget === "specific" && (
                <label>
                  <span>任务 ID</span>
                  <input
                    aria-label="指定任务 ID"
                    type="text"
                    value={data.specificIssueId ?? ""}
                    placeholder="例如 LOCAL-48"
                    onChange={(event) => onChange({ specificIssueId: event.target.value })}
                  />
                </label>
              )}
              <div className="workflow-action-row">
                <label className="workflow-action-toggle">
                  <input
                    type="checkbox"
                    checked={data.changeStatus ?? false}
                    onChange={(event) => onChange({ changeStatus: event.target.checked })}
                  />
                  <span>改变状态</span>
                </label>
                <select
                  aria-label="目标状态"
                  disabled={!data.changeStatus}
                  value={data.targetStatus ?? "in_review"}
                  onChange={(event) => onChange({ targetStatus: event.target.value })}
                >
                  {ISSUE_STATUSES.map((status) => (
                    <option key={status.value} value={status.value}>{status.label}</option>
                  ))}
                </select>
              </div>
              <div className="workflow-action-row">
                <label className="workflow-action-toggle">
                  <input
                    type="checkbox"
                    checked={data.addComment ?? false}
                    onChange={(event) => onChange({ addComment: event.target.checked })}
                  />
                  <span>添加评论</span>
                </label>
                <select
                  aria-label="评论内容"
                  disabled={!data.addComment}
                  value={data.commentSource ?? "workflow-output"}
                  onChange={(event) => onChange({ commentSource: event.target.value })}
                >
                  <option value="workflow-output">上游节点输出</option>
                  <option value="run-summary">流程运行摘要</option>
                  <option value="custom">自定义内容</option>
                </select>
              </div>
              {data.addComment && data.commentSource === "custom" && (
                <label>
                  <span>评论内容</span>
                  <textarea
                    rows={3}
                    value={data.customComment ?? ""}
                    placeholder="输入要追加到任务的评论…"
                    onChange={(event) => onChange({ customComment: event.target.value })}
                  />
                </label>
              )}
              <div className="workflow-action-row">
                <label className="workflow-action-toggle">
                  <input
                    type="checkbox"
                    checked={data.addLabels ?? false}
                    onChange={(event) => onChange({ addLabels: event.target.checked })}
                  />
                  <span>添加标签</span>
                </label>
                <input
                  aria-label="要添加的标签"
                  type="text"
                  disabled={!data.addLabels}
                  value={data.labelsToAdd ?? ""}
                  placeholder="自动化, 已处理"
                  onChange={(event) => onChange({ labelsToAdd: event.target.value })}
                />
              </div>
              <div className="workflow-action-row">
                <label className="workflow-action-toggle">
                  <input
                    type="checkbox"
                    checked={data.setPriority ?? false}
                    onChange={(event) => onChange({ setPriority: event.target.checked })}
                  />
                  <span>设置优先级</span>
                </label>
                <select
                  aria-label="目标优先级"
                  disabled={!data.setPriority}
                  value={data.targetPriority ?? "none"}
                  onChange={(event) => onChange({ targetPriority: event.target.value })}
                >
                  {ISSUE_PRIORITIES.map((priority) => (
                    <option key={priority.value} value={priority.value}>{priority.label}</option>
                  ))}
                </select>
              </div>
              <label className="workflow-action-toggle workflow-action-toggle-full">
                <input
                  type="checkbox"
                  checked={data.attachArtifacts ?? false}
                  onChange={(event) => onChange({ attachArtifacts: event.target.checked })}
                />
                <span>附加流程运行产物</span>
              </label>
              <label className="workflow-action-toggle workflow-action-toggle-full">
                <input
                  type="checkbox"
                  checked={data.recordConversation ?? false}
                  onChange={(event) => onChange({ recordConversation: event.target.checked })}
                />
                <span>记录执行该任务的 Codex 对话</span>
              </label>
            </div>
          )}

          <div className="workflow-config-section">
            <h2>连接</h2>
            <div className="workflow-port-row">
              <span><i className="input" aria-hidden="true" />输入</span>
              <strong>{data.inputLabel ?? "无"}</strong>
            </div>
            <div className="workflow-port-row">
              <span><i className="output" aria-hidden="true" />输出</span>
              <strong>{data.outputLabel ?? "无"}</strong>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
