# 外部能力安装脚本（Phase0）

对应 [Phase0 契约 §4](../../docs/phase0-contract.md)。每个 capability 一个 `scripts/install/<capability>.sh`，
名称与 `mathscape-agent` 的 `CapabilityProbe.install_hint` 一致。约定：

- 幂等、可重跑；只装到用户目录 / venv，不改系统全局。
- 缺失是正常状态：探测（`CapabilityProbe`）只读，不会自动安装。
- Windows `.ps1` 与离线 sidecar 包后补。

| 脚本 | 状态 | 说明 |
| --- | --- | --- |
| `marker.sh` | 可用 | 在 `${XDG_DATA_HOME:-~/.local/share}/mathscape/marker/venv` 创建 venv 并安装 `marker-pdf>=2,<3`（需 Python 3.10+，会下载 PyTorch / 模型）。`--check` 只检测，`--dry-run` 只打印，`--upgrade` 升级。可用 `MATHSCAPE_MARKER_VENV`、`PYTHON`、`PIP_INDEX_URL` 覆盖。 |
| `lean.sh` | 桩 | 只检测 `lean` 并打印 elan 官方安装提示。 |
| `codex.sh` | 桩 | 只检测 `codex` 并打印安装提示。 |
| `claude_code.sh` | 桩 | 只检测 `claude` 并打印安装提示。 |
| `hmz.sh` | 桩 | 可选适配器；缺失不影响任何默认流程。 |

Marker 安装后，桌面端仍按 [README](../../README.md) 通过 `MATHSCAPE_MARKER_EXECUTABLE` 指向
`<venv>/bin/marker_single`；`CapabilityProbe` 也会自动探测上述默认 venv。

```sh
scripts/install/marker.sh --check   # available / version / missing
scripts/install/marker.sh           # 安装或确认已安装
```
