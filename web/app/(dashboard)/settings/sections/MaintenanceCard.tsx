"use client";

import { useState } from "react";
import { useDialogs } from "@/app/components/Dialogs";
import {
  BrutalCard,
  Field,
  StatusBadge,
  buttonClass,
  inputClass,
  responseError,
} from "@/app/components/M7Primitives";
import {
  apiClient,
  type MaintenanceRestoreResponse,
  type MaintenanceStatusResponse,
  type TsdbCompactResponse,
  type TsdbRetentionResponse,
} from "@/lib/api";
import { useI18n } from "@/lib/use-i18n";
import { useLoadOnVisible } from "@/app/lib/use-load-on-visible";
import { MaintenanceCapability, SectionFeedback, saveBlob } from "./shared";
import { useSensitiveTotp } from "./use-sensitive-totp";

export function MaintenanceCard() {
  const dialogs = useDialogs();
  const { t: copy } = useI18n();
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const sensitiveTotpCode = useSensitiveTotp(dialogs, setError);
  const [maintenanceStatus, setMaintenanceStatus] = useState<MaintenanceStatusResponse | null>(null);
  const [maintenanceLoading, setMaintenanceLoading] = useState(true);
  const [restoreFile, setRestoreFile] = useState<File | null>(null);
  const [restoreResult, setRestoreResult] = useState<MaintenanceRestoreResponse | null>(null);
  const [tsdbCompactResult, setTsdbCompactResult] = useState<TsdbCompactResponse | null>(null);
  const [tsdbRetentionResult, setTsdbRetentionResult] = useState<TsdbRetentionResponse | null>(null);
  const [tsdbRetentionDraft, setTsdbRetentionDraft] = useState("30");
  const cardRef = useLoadOnVisible(() => void loadMaintenanceStatus());

  async function loadMaintenanceStatus() {
    setMaintenanceLoading(true);
    const response = await apiClient.getMaintenanceStatus();
    setMaintenanceLoading(false);
    if (response.success && response.data) {
      setMaintenanceStatus(response.data);
      if (response.data.tsdb_retention_days) {
        setTsdbRetentionDraft(String(response.data.tsdb_retention_days));
      }
    } else {
      setError(responseError(response));
    }
  }

  async function runSqliteVacuum() {
    if (!(await dialogs.confirm({ message: copy.settingsPage.vacuumConfirm, danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setMaintenanceLoading(true);
    const response = await apiClient.vacuumSqlite(totpCode);
    setMaintenanceLoading(false);
    if (response.success) {
      setNotice(copy.settingsPage.vacuumDone);
      await loadMaintenanceStatus();
    } else {
      setError(responseError(response));
    }
  }

  async function runTsdbCompact() {
    if (!(await dialogs.confirm({ message: copy.settingsPage.tsdbCompactConfirm, danger: true }))) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setMaintenanceLoading(true);
    const response = await apiClient.compactTsdb(totpCode);
    setMaintenanceLoading(false);
    if (response.success && response.data) {
      setTsdbCompactResult(response.data);
      setNotice(copy.settingsPage.tsdbCompactDone.replace("{count}", String(response.data.removed_samples)));
      await loadMaintenanceStatus();
    } else {
      setError(responseError(response));
    }
  }

  async function updateTsdbRetention() {
    const parsed = Number.parseInt(tsdbRetentionDraft.trim(), 10);
    if (!Number.isFinite(parsed) || parsed < 1 || parsed > 3650) {
      setError(copy.settingsPage.tsdbRetentionRange);
      return;
    }
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setMaintenanceLoading(true);
    const response = await apiClient.updateTsdbRetention(parsed, totpCode);
    setMaintenanceLoading(false);
    if (response.success && response.data) {
      setTsdbRetentionResult(response.data);
      setNotice(copy.settingsPage.tsdbRetentionUpdated.replace("{days}", String(response.data.retention_days)));
      await loadMaintenanceStatus();
    } else {
      setError(responseError(response));
    }
  }

  async function downloadMaintenanceExport(kind: "backup" | "archive") {
    const supported = kind === "backup" ? maintenanceStatus?.backup_supported : maintenanceStatus?.archive_supported;
    if (!supported) return;
    const totpCode = await sensitiveTotpCode();
    if (totpCode === null) return;
    setMaintenanceLoading(true);
    const response = kind === "backup"
      ? await apiClient.downloadMaintenanceBackup(totpCode)
      : await apiClient.downloadMaintenanceArchive(totpCode);
    setMaintenanceLoading(false);
    if (response.success && response.data) {
      saveBlob(response.data.blob, response.data.filename);
      setNotice(kind === "backup" ? copy.settingsPage.backupDownloadStarted : copy.settingsPage.archiveDownloadStarted);
    } else {
      setError(responseError(response));
    }
  }

  async function restoreSqliteBackup(dryRun: boolean) {
    if (!restoreFile) {
      setError(copy.settingsPage.selectSqliteBackup);
      return;
    }
    if (!maintenanceStatus?.restore_supported) {
      setError(copy.settingsPage.restoreUnsupported);
      return;
    }
    if (!dryRun && !(await dialogs.confirm({ message: copy.settingsPage.restoreConfirm, danger: true }))) return;
    const totpCode = dryRun ? undefined : await sensitiveTotpCode();
    if (totpCode === null) return;
    setMaintenanceLoading(true);
    const response = await apiClient.restoreBackup(restoreFile, dryRun, totpCode);
    setMaintenanceLoading(false);
    if (response.success && response.data) {
      setRestoreResult(response.data);
      setNotice(dryRun ? copy.settingsPage.backupVerified : copy.settingsPage.backupRestored);
      await loadMaintenanceStatus();
    } else {
      setError(responseError(response));
    }
  }

  return (
    <div ref={cardRef} className="mb-6">
      <BrutalCard>
        <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
          <div>
            <h2 className="text-xl font-black uppercase">{copy.settingsPage.backupAndMaintenance}</h2>
          </div>
          <button type="button" className={buttonClass("secondary")} onClick={() => void loadMaintenanceStatus()} disabled={maintenanceLoading}>
            {maintenanceLoading ? copy.settingsPage.loading : copy.settingsPage.refresh}
          </button>
        </div>
        <SectionFeedback error={error} notice={notice} />
        <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_auto] xl:items-end">
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-8">
            <div className="border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)]">
              <div className="text-[11px] font-black uppercase text-[var(--text-muted)]">{copy.settingsPage.database}</div>
              <div className="mt-2 text-lg font-black">{maintenanceStatus?.database_backend ?? "N/A"}</div>
            </div>
            <div className="border-2 border-black bg-[var(--accent-bg)] p-3 shadow-[var(--shadow-brutal-sm)]">
              <div className="text-[11px] font-black uppercase text-[var(--text-muted)]">{copy.settingsPage.tsdb}</div>
              <div className="mt-2 text-sm font-black">{maintenanceStatus?.tsdb_backend ?? "N/A"}</div>
              <div className="mt-1 text-xs font-bold text-[var(--text-muted)]">
                {maintenanceStatus?.tsdb_status ?? "unknown"} / {maintenanceStatus?.tsdb_samples ?? "N/A"} samples
              </div>
              <div className="mt-1 text-xs font-bold text-[var(--text-muted)]">
                {copy.settingsPage.retentionDays.replace("{days}", String(maintenanceStatus?.tsdb_retention_days ?? "N/A"))}
              </div>
            </div>
            <MaintenanceCapability label={copy.settingsPage.backupDownload} enabled={maintenanceStatus ? Boolean(maintenanceStatus.backup_supported) : undefined} copy={copy} />
            <MaintenanceCapability label={copy.settingsPage.fullArchive} enabled={maintenanceStatus ? Boolean(maintenanceStatus.archive_supported) : undefined} copy={copy} />
            <MaintenanceCapability label={copy.settingsPage.backupRestore} enabled={maintenanceStatus ? Boolean(maintenanceStatus.restore_supported) : undefined} copy={copy} />
            <MaintenanceCapability label={copy.settingsPage.sqliteVacuum} enabled={maintenanceStatus ? Boolean(maintenanceStatus.vacuum_supported) : undefined} copy={copy} />
            <MaintenanceCapability label={copy.settingsPage.tsdbCompact} enabled={maintenanceStatus ? Boolean(maintenanceStatus.tsdb_compact_supported) : undefined} copy={copy} />
            <MaintenanceCapability label={copy.settingsPage.tsdbRetention} enabled={maintenanceStatus ? Boolean(maintenanceStatus.tsdb_retention_configurable) : undefined} copy={copy} />
          </div>
          <div className="flex flex-wrap gap-2 xl:justify-end">
            <input
              className={`${inputClass} w-32`}
              value={tsdbRetentionDraft}
              onChange={(event) => setTsdbRetentionDraft(event.target.value)}
              inputMode="numeric"
              aria-label={copy.settingsPage.tsdbRetentionDaysAria}
              disabled={!maintenanceStatus?.tsdb_retention_configurable || maintenanceLoading}
            />
            <button
              type="button"
              className={buttonClass("secondary")}
              onClick={() => void updateTsdbRetention()}
              disabled={!maintenanceStatus?.tsdb_retention_configurable || maintenanceLoading}
            >
              {copy.settingsPage.saveRetention}
            </button>
            <button
              type="button"
              className={buttonClass(maintenanceStatus?.backup_supported ? "primary" : "secondary")}
              disabled={!maintenanceStatus?.backup_supported || maintenanceLoading}
              onClick={() => void downloadMaintenanceExport("backup")}
            >
              {copy.settingsPage.downloadBackup}
            </button>
            <button
              type="button"
              className={buttonClass(maintenanceStatus?.archive_supported ? "primary" : "secondary")}
              disabled={!maintenanceStatus?.archive_supported || maintenanceLoading}
              onClick={() => void downloadMaintenanceExport("archive")}
            >
              {copy.settingsPage.downloadArchive}
            </button>
            <button
              type="button"
              className={buttonClass("secondary")}
              onClick={() => void runSqliteVacuum()}
              disabled={!maintenanceStatus?.vacuum_supported || maintenanceLoading}
            >
              {copy.settingsPage.runVacuum}
            </button>
            <button
              type="button"
              className={buttonClass("secondary")}
              onClick={() => void runTsdbCompact()}
              disabled={!maintenanceStatus?.tsdb_compact_supported || maintenanceLoading}
            >
              {copy.settingsPage.runTsdbCompact}
            </button>
          </div>
        </div>
        <div className="mt-4 grid gap-3 border-t-2 border-black pt-4 xl:grid-cols-[minmax(0,1fr)_auto] xl:items-end">
          <Field label={copy.settingsPage.sqliteBackupFile}>
            <input
              className={inputClass}
              type="file"
              accept=".sqlite,.sqlite3,.db,application/vnd.sqlite3,application/x-sqlite3"
              onChange={(event) => {
                setRestoreFile(event.target.files?.[0] ?? null);
                setRestoreResult(null);
              }}
            />
          </Field>
          <div className="flex flex-wrap gap-2 xl:justify-end">
            <button
              type="button"
              className={buttonClass("secondary")}
              onClick={() => void restoreSqliteBackup(true)}
              disabled={!maintenanceStatus?.restore_supported || !restoreFile || maintenanceLoading}
            >
              {copy.settingsPage.verifyBackup}
            </button>
            <button
              type="button"
              className={buttonClass("danger")}
              onClick={() => void restoreSqliteBackup(false)}
              disabled={!maintenanceStatus?.restore_supported || !restoreFile || maintenanceLoading}
            >
              {copy.settingsPage.restoreBackupAction}
            </button>
          </div>
        </div>
        {restoreResult ? (
          <div className="mt-3 flex flex-wrap gap-2 text-xs font-bold text-[var(--text-muted)]">
            <StatusBadge tone={restoreResult.restored ? "green" : "blue"}>{restoreResult.restored ? copy.settingsPage.restored : copy.settingsPage.verified}</StatusBadge>
            <span>{copy.settingsPage.tableCount.replace("{count}", String(restoreResult.table_count))}</span>
            <span>{copy.settingsPage.rowCount.replace("{count}", String(restoreResult.row_count))}</span>
            <span>{copy.settingsPage.userVersion.replace("{version}", String(restoreResult.user_version))}</span>
          </div>
        ) : null}
        {tsdbCompactResult ? (
          <div className="mt-3 flex flex-wrap gap-2 text-xs font-bold text-[var(--text-muted)]">
            <StatusBadge tone="green">{copy.settingsPage.tsdbCompact}</StatusBadge>
            <span>{copy.settingsPage.removedSamples.replace("{count}", String(tsdbCompactResult.removed_samples))}</span>
            <span>{copy.settingsPage.samplesBefore.replace("{count}", String(tsdbCompactResult.samples_before ?? "N/A"))}</span>
            <span>{copy.settingsPage.samplesAfter.replace("{count}", String(tsdbCompactResult.samples_after ?? "N/A"))}</span>
          </div>
        ) : null}
        {tsdbRetentionResult ? (
          <div className="mt-3 flex flex-wrap gap-2 text-xs font-bold text-[var(--text-muted)]">
            <StatusBadge tone="green">{copy.settingsPage.tsdbRetention}</StatusBadge>
            <span>{copy.settingsPage.daysCount.replace("{days}", String(tsdbRetentionResult.retention_days))}</span>
            <span>{copy.settingsPage.samplesBefore.replace("{count}", String(tsdbRetentionResult.samples_before ?? "N/A"))}</span>
            <span>{copy.settingsPage.samplesAfter.replace("{count}", String(tsdbRetentionResult.samples_after ?? "N/A"))}</span>
          </div>
        ) : null}
        <p className="mt-3 text-xs font-bold text-[var(--text-muted)]">
          {copy.settingsPage.maintenanceHint}
        </p>
      </BrutalCard>
      {dialogs.element}
    </div>
  );
}
