`unsaved-operations-epoch-conflict.json` is a real A1 "导出到文件…" export
(`mathscape-ink-unsaved-operations`, PR #4 regression report): one stroke from
writer epoch 1 quarantined with `writer_epoch_conflict` after a newer writer
appended history. Import must rebuild it as a recovery draft.
