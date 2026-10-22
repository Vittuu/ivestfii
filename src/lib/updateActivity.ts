import { call, desktop } from './bridge';

export function pendingUpdateWork(): string[] {
  const reasons: string[] = [];
  if (document.querySelector('dialog[open]')) reasons.push('Feche ou conclua a janela de edição aberta.');
  if (document.querySelector('.settings-pending')) reasons.push('Salve as alterações pendentes nas configurações ou no perfil.');
  if (document.querySelector('form.market-mapping')) reasons.push('Salve ou cancele a edição do CNPJ no Mercado.');
  if (document.querySelector<HTMLTextAreaElement>('#chat-draft')?.value.trim()) reasons.push('Envie ou limpe o rascunho do assistente.');
  if (Array.from(document.querySelectorAll<HTMLInputElement>('input[type="password"]')).some((input) => input.value.length > 0)) reasons.push('Salve ou limpe a chave digitada nas configurações.');
  return reasons;
}

let activityReport: Promise<unknown> = Promise.resolve();
let lastReported: boolean | undefined;
/** Serialize with the monitor so an older report cannot overwrite this fresh check. */
export function syncUpdateActivity(): Promise<string[]> {
  const report = activityReport.catch(() => {}).then(async () => {
    const reasons = pendingUpdateWork();
    if (desktop) await call('set_update_activity', { blocked: reasons.length > 0 });
    lastReported = reasons.length > 0;
    return reasons;
  });
  activityReport = report;
  return report;
}

/** Reports only whether restarting would discard work. Field contents stay in the WebView. */
export function startUpdateActivityMonitor(): () => void {
  if (!desktop) return () => {};
  let stopped = false;
  let timer: number | undefined;
  let inFlight = false;
  lastReported = undefined;
  const blocked = () => pendingUpdateWork().length > 0;
  const schedule = () => {
    if (stopped) return;
    // Coalesce without postponing forever when progress/status DOM keeps changing.
    if (timer !== undefined) return;
    timer = window.setTimeout(() => { timer = undefined; void report(); }, 120);
  };
  const report = async () => {
    if (stopped) return;
    if (inFlight) { schedule(); return; }
    const next = blocked();
    if (lastReported === next) return;
    inFlight = true;
    try {
      await syncUpdateActivity();
    } catch { /* The periodic check retries IPC failures without exposing contents. */ }
    finally { inFlight = false; }
    if (!stopped && next !== blocked()) schedule();
  };
  const observer = new MutationObserver(schedule);
  observer.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['open', 'class'] });
  document.addEventListener('input', schedule, true);
  document.addEventListener('change', schedule, true);
  schedule();
  const retry = window.setInterval(schedule, 1_000);
  return () => {
    stopped = true;
    window.clearTimeout(timer);
    window.clearInterval(retry);
    observer.disconnect();
    document.removeEventListener('input', schedule, true);
    document.removeEventListener('change', schedule, true);
  };
}
