# LLM Capability Doctor（大模型能力诊断工具）

> 面向智能体平台模型接入场景的一站式、全维度大模型适配性诊断工具。

## 工具开发背景

在智能体平台版本的实际对接落地过程中，客户侧接入的大模型类型繁杂、能力参差不齐，无法提前确保模型能够被智能体平台正常兼容调用。

为高效、标准化地核验客户大模型的适配性，特开发本诊断工具，实现一站式、全维度检测。

## 核心使用流程

仅需一个检测脚本，配合本地预装的专属 Skill 工具，即可完成全流程操作：

1. **拷贝检测脚本**：将单个检测脚本拷贝至客户现场。
2. **执行自动检测**：运行一条命令，启动全自动检测。
3. **导出执行日志**：检测完成后，导出完整执行日志并传回本地。
4. **生成诊断报告**：通过自研 Skill 工具解析日志，一键生成标准化的大模型体检 HTML 报告。

> **全流程仅需两步：执行脚本、解析日志。**

依托可视化检测报告，可将诊断结果作为客观依据，与客户进行高效沟通。

## 设计核心考量

| 核心考量 | 设计说明 |
| --- | --- |
| **检测维度全面** | 覆盖网络连通性、上下文长度上限、并发性能、工具调用能力、思维链能力、敏感词过滤策略等关键指标。 |
| **现场使用极简** | 仅需执行脚本、解析日志，无需复杂部署与配置。 |
| **结果分级清晰** | 区分核心必过检测项与次要优化检查项，重点突出。 |
| **报告证据完整** | 清晰呈现每项检测的逻辑说明、输入参数与实际输出结果，确保结论客观、可追溯。 |

## 如何执行模型doctor检测

1.将model-capability-doctor.sh这个脚本传入客户服务器，执行下面的命令：
```bash
./model-capability-doctor.sh \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --api-key 'xxxxxxxxxx' \
  --log-file './contract-test.log'
```
2.执行完成后将客户现场的contract-test.log拷贝出来
3.在自己电脑安装model doctor的skill，将下面这句话复制给codex执行
```bash
帮我安装https://github.com/relaxcloud-cn/llm-capability-doctor/tree/main/skills/creating-model-doctor-reports到本地
```
4.在codex输入这句话，使用skill分析日志得到模型体检的html报告
```
使用model doctor report技能分析下 contract-test.log
```
