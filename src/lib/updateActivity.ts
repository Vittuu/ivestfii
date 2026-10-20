import { call, desktop } from './bridge';

/** Reports only whether restarting would discard work. Field contents stay in the WebView. */
export function startUpdateActivityMonitor(): () => void {
  if (!desktop) return () => {};
  let stopped = false;
  let timer: number | undefined;
  let inFlight = false;
  let lastReported: boolean | undefined;
  const blocked = () => Boolean(
    document.querySelector('dialog[open], .settings-pending, form.market-mapping')
    || document.querySelector<HTMLTextAreaElement>('#chat-draft')?.value.trim()
    || Array.from(document.querySelectorAll<HTMLInputElement>('input[type="password"]')).some((input) => input.value.length > 0),
  );
  const schedule = () => {
    if (stopped) return;
    window.clearTimeout(timer);
    timer = window.setTimeout(() => void report(), 120);
  };
  const report = async () => {
    if (stopped) return;
    if (inFlight) { schedule(); return; }
    const next = blocked();
    if (lastReported === next) return;
    inFlight = true;
    try {
      await call('set_update_activity', { blocked: next });
      if (!stopped) lastReported = next;
    } catch { /* Retry on the next activity change; there is no content to log. */ }
    finally { inFlight = false; }
    if (!stopped && next !== blocked()) schedule();
  };
  const observer = new MutationObserver(schedule);
  observer.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['open', 'class'] });
  document.addEventListener('input', schedule, true);
  document.addEventListener('change', schedule, true);
  schedule();
  return () => {
    stopped = true;
    window.clearTimeout(timer);
    observer.disconnect();
    document.removeEventListener('input', schedule, true);
    document.removeEventListener('change', schedule, true);
  };
}
