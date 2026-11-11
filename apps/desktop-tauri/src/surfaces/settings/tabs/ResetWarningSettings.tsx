import { useState } from "react";
import { Field, Select, Toggle } from "../../../components/FormControls";
import { useLocale } from "../../../hooks/useLocale";
import { testSessionResetWarning } from "../../../lib/tauri";
import type { TabProps } from "../settingsTabs";

const LEAD_MINUTES = [5, 10, 15, 30, 60];

export default function ResetWarningSettings({ settings, set, saving }: TabProps) {
  const { t } = useLocale();
  const enabled = settings.sessionResetWarningEnabled ?? true;
  const savedTopic = settings.sessionResetWarningNtfyTopic ?? "";
  const [draftTopic, setDraftTopic] = useState<string | null>(null);
  const [testing, setTesting] = useState(false);
  const [status, setStatus] = useState("");
  const [error, setError] = useState("");
  const topic = (draftTopic ?? savedTopic).trim();

  function saveTopic() {
    setDraftTopic(null);
    if (topic !== savedTopic) set({ sessionResetWarningNtfyTopic: topic });
  }

  async function sendTest(channel: "desktop" | "ntfy") {
    setTesting(true);
    setStatus("");
    setError("");
    try {
      await testSessionResetWarning(channel === "ntfy" ? topic : undefined);
      setStatus(
        t(channel === "ntfy" ? "SessionResetWarningNtfySent" : "SessionResetWarningDesktopSent"),
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setTesting(false);
    }
  }

  return (
    <>
      <Field
        label={t("SessionResetWarningEnabled")}
        description={t("SessionResetWarningHelper")}
      >
        <Toggle
          variant="switch"
          checked={enabled}
          ariaLabel={t("SessionResetWarningEnabled")}
          disabled={saving}
          onChange={(value) => set({ sessionResetWarningEnabled: value })}
        />
      </Field>
      <Field label={t("SessionResetWarningLead")}>
        <Select
          variant="menu"
          value={String(settings.sessionResetWarningLeadMinutes ?? 30)}
          options={LEAD_MINUTES.map((minutes) => ({
            value: String(minutes),
            label: t("SessionResetWarningMinutes").replace("{}", String(minutes)),
          }))}
          ariaLabel={t("SessionResetWarningLead")}
          disabled={saving || !enabled}
          onChange={(value) => set({ sessionResetWarningLeadMinutes: Number(value) })}
        />
      </Field>
      <Field label={t("SessionResetWarningNtfyTopic")} description={t("SessionResetWarningNtfyHelper")}>
        <div className="settings-group__row">
          <input
            type="text"
            className="text-input"
            value={draftTopic ?? savedTopic}
            aria-label={t("SessionResetWarningNtfyTopic")}
            autoComplete="off"
            spellCheck={false}
            maxLength={64}
            disabled={saving}
            onChange={(event) => setDraftTopic(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") saveTopic();
            }}
          />
          <button
            type="button"
            className="credential-btn"
            disabled={saving || topic === savedTopic}
            onClick={saveTopic}
          >
            {t("Save")}
          </button>
        </div>
      </Field>
      <div className="settings-group__row">
        <button
          type="button"
          className="credential-btn"
          disabled={testing}
          onClick={() => void sendTest("desktop")}
        >
          {t("SessionResetWarningDesktopTest")}
        </button>
        <button
          type="button"
          className="credential-btn"
          disabled={testing || !topic}
          onClick={() => void sendTest("ntfy")}
        >
          {t("SessionResetWarningNtfyTest")}
        </button>
        {status && <span role="status">{status}</span>}
        {error && <p className="settings-group__error" role="alert">{error}</p>}
      </div>
    </>
  );
}
