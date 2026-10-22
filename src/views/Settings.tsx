import { useEffect, useId, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { ArrowDownToLine, ArrowUpFromLine, LoaderCircle, RefreshCw, Save } from 'lucide-react';
import { call, desktop, errorMessage } from '../lib/bridge';
import type { AppInfo, MarketDataSettings, Settings as AppSettings, UpdateStatus } from '../lib/bridge';
import { Def, Field, SectionHeading } from '../components';
import PublicSourceLink from '../components/PublicSourceLink';
import { readPublicCache, usePublicData } from '../lib/publicData';
import { InvestmentProfileSection } from '../components/InvestmentProfileForm';

export default function Settings({ walletId, walletName, info, onImport, onExport, onImportPosition, notify, onOnboarding }: { walletId: string; walletName: string; info: AppInfo | null; onImport: () => void; onExport: () => void; onImportPosition: () => void; notify: (message: string) => void; onOnboarding: () => void }) {
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [initial, setInitial] = useState<AppSettings | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => { if (!desktop) return; void call<AppSettings>('get_settings').then((result) => { setSettings(result); setInitial(result); }).catch((err) => setError(errorMessage(err))); }, []);
  const save = async () => {
    if (!settings) return;
    setBusy(true); setError('');
    try { await call('save_settings', { settings }); setInitial({ ...settings }); notify('Preferências do aplicativo salvas.'); }
    catch (err) { setError(errorMessage(err)); } finally { setBusy(false); }
  };
  const changed = JSON.stringify(settings) !== JSON.stringify(initial);
  const locked = !settings || busy;
  const saveStatus = changed ? 'Alterações não salvas.' : !desktop ? 'Disponível no aplicativo Windows.' : !settings ? (error ? 'Preferências indisponíveis.' : 'Carregando preferências…') : 'Sem alterações.';
  const syncStatus = !info || info.sync.status === 'not_configured' ? 'Não configurada' : info.sync.status;
  return <div className="page settings-page">
    <section className="group" aria-labelledby="settings-onboarding"><div className="group-head"><h2 id="settings-onboarding">Primeiros passos</h2></div><div className="block"><ul className="settings-actions"><ActionRow title="Comece com o básico" description="Uma configuração curta, com opção de deixar informações para depois. Use o assistente para tirar dúvidas.">{() => <button className="button" type="button" disabled={!desktop} onClick={onOnboarding}>Rever primeiros passos</button>}</ActionRow></ul></div></section>
    <section className="group" aria-labelledby="settings-app">
      <div className="group-head"><h2 id="settings-app">Aplicativo</h2><p>Vale para este computador</p></div>
      <div className="modular">
        <div className="block span-12 settings-main">
          <ul className="settings-list">
            <PreferenceRow title="Iniciar com o Windows" description="Abre o IvestFII ao fazer login." checked={settings?.autostart || false} disabled={locked} onChange={(checked) => setSettings((old) => old && ({ ...old, autostart: checked }))} />
            <PreferenceRow sub title="Iniciar na bandeja" description="Ao abrir com o Windows, a janela começa recolhida." checked={settings?.startMinimized || false} disabled={locked} onChange={(checked) => setSettings((old) => old && ({ ...old, startMinimized: checked }))} />
            <PreferenceRow title="Fechar para a bandeja" description="O botão X recolhe a janela. Use Sair, na bandeja, para encerrar." checked={settings?.closeToTray || false} disabled={locked} onChange={(checked) => setSettings((old) => old && ({ ...old, closeToTray: checked }))} />
            <PreferenceRow title="Atualizações automáticas" description="Busca e prepara novas versões deste aplicativo." checked={settings?.autoUpdate ?? true} disabled={locked} onChange={(checked) => setSettings((old) => old && ({ ...old, autoUpdate: checked }))} />
          </ul>
          {error && <p className="form-error" role="alert">{error}</p>}
          <div className="block-foot settings-save">
            <span className={changed ? 'settings-pending' : undefined} aria-live="polite">{saveStatus}</span>
            <button className={`button small${changed ? ' primary' : ''}`} onClick={() => void save()} disabled={!settings || busy || !changed}>{busy ? <LoaderCircle className="spin" size={16} /> : <Save size={16} />}Salvar preferências</button>
          </div>
        </div>
      </div>
      <details className="extra-details settings-extra"><summary>Privacidade e sincronização</summary>
        <div className="settings-extra-body">
          <dl className="facts settings-facts">
            <div><dt>Análises de IA</dt><dd className="settings-fact-text">Só quando você pede</dd></div>
            <div><dt>Conversas do assistente</dt><dd className="settings-fact-text">Banco local</dd></div>
            <div><dt>Conexão com o servidor</dt><dd className="settings-fact-text">{syncStatus}</dd></div>
            <div><dt>Alterações locais pendentes</dt><dd>{info?.sync.pendingChanges ?? '—'}</dd></div>
          </dl>
          <p className="settings-note">A sincronização com a Vercel ainda não está disponível.</p>
        </div>
      </details>
    </section>

    <UpdateSettings />

    <InvestmentProfileSection walletId={walletId} walletName={walletName} />

    <MarketSettings />

    <section className="group" aria-labelledby="settings-data">
      <div className="group-head"><h2 id="settings-data">Dados locais</h2><p>Um banco por carteira</p></div>
      <div className="modular">
        <div className="block span-12">
          <SectionHeading title="Importar e exportar" level={3} />
          <ul className="settings-actions">
            <ActionRow title="Posição da corretora" description="Importa a posição ou um complemento do pós-fixado com seus aportes. O histórico não muda.">{(describedBy) => <button className="button" onClick={onImportPosition} disabled={!desktop} aria-describedby={describedBy}><ArrowDownToLine size={16} />Importar posição</button>}</ActionRow>
            <ActionRow title="Histórico de lançamentos" description="Aceita o arquivo antigo ou um backup do IvestFII, com a posição incluída. Repetições não duplicam registros.">{(describedBy) => <button className="button" onClick={onImport} disabled={!desktop} aria-describedby={describedBy}><ArrowDownToLine size={16} />Importar histórico ou backup</button>}</ActionRow>
            <ActionRow title="Backup da carteira" description="FIIs, caixinhas e posição da carteira selecionada. Exporte uma carteira por vez.">{(describedBy) => <button className="button" onClick={onExport} disabled={!desktop} aria-describedby={describedBy}><ArrowUpFromLine size={16} />Exportar carteira selecionada</button>}</ActionRow>
          </ul>
          {Boolean(info?.positionImport?.error) && <div className="form-error" role="alert">A posição completa não pôde ser importada: {String(info?.positionImport?.error)}. Corrija o arquivo e use Importar posição.</div>}
        </div>
      </div>
      <details className="extra-details settings-extra" open={Boolean(info?.migration?.error)}><summary>Arquivos e diagnóstico</summary>
          <dl className="settings-paths">
            {info?.dataDirectory && <PathRow label="Pasta de dados"><code>{info.dataDirectory}</code></PathRow>}
            <PathRow label="Banco de dados">{info?.databasePath ? <code>{info.databasePath}</code> : <span className="muted">{desktop ? 'Carregando localização…' : 'Indisponível nesta prévia web'}</span>}</PathRow>
            {info?.positionPath && <PathRow label="Arquivo de posição"><code>{info.positionPath}</code></PathRow>}
            {info?.legacyPath && <PathRow label="Arquivo anterior"><code>{info.legacyPath}</code></PathRow>}
            {info?.migration && <MigrationStatus migration={info.migration} />}
          </dl>
      </details>
    </section>
  </div>;
}

function UpdateSettings() {
  const [update, setUpdate] = useState<UpdateStatus | null>(null);
  const [action, setAction] = useState<'check' | 'install' | null>(null);
  const [error, setError] = useState('');
  const [readError, setReadError] = useState('');
  const mounted = useRef(false);
  const actionPending = useRef(false);
  const reading = useRef(false);
  const generation = useRef(0);
  const previousStatus = useRef<string | null>(null);
  useEffect(() => {
    mounted.current = true;
    if (!desktop) return () => { mounted.current = false; };
    const refresh = async () => {
      if (actionPending.current || reading.current) return;
      reading.current = true;
      const current = generation.current;
      try {
        const result = await call<UpdateStatus>('get_update_status');
        if (mounted.current && generation.current === current) {
          if (previousStatus.current !== result.status) setError('');
          previousStatus.current = result.status;
          setUpdate(result); setReadError('');
        }
      } catch (err) { if (mounted.current && generation.current === current) setReadError(errorMessage(err)); }
      finally { reading.current = false; }
    };
    void refresh();
    const timer = window.setInterval(() => void refresh(), 2_000);
    return () => { mounted.current = false; window.clearInterval(timer); };
  }, []);
  const run = async (kind: 'check' | 'install') => {
    if (actionPending.current) return;
    actionPending.current = true; generation.current += 1; setAction(kind); setError(''); setReadError('');
    try {
      const result = await call<UpdateStatus>(kind === 'check' ? 'check_for_updates' : 'install_pending_update');
      if (mounted.current) { previousStatus.current = result.status; setUpdate(result); }
    } catch (err) { if (mounted.current) setError(errorMessage(err)); }
    finally { actionPending.current = false; if (mounted.current) setAction(null); }
  };
  const working = ['checking', 'available', 'downloading', 'installing'].includes(update?.status || '');
  const unavailable = !desktop || (!update && !readError && !error) || update?.status === 'disabled';
  const status = !desktop ? 'Disponível no aplicativo Windows.' : readError ? 'Não foi possível consultar as atualizações.' : error && !['ready', 'waiting_for_idle'].includes(update?.status || '') ? 'Não foi possível concluir a ação de atualização.' : !update ? 'Consultando atualizações…' : updateMessage(update);
  const total = update?.totalBytes && update.totalBytes > 0 ? update.totalBytes : null;
  const downloaded = Math.max(0, update?.downloadedBytes || 0);
  const percent = total ? Math.min(100, Math.round(downloaded / total * 100)) : null;
  return <section className="group" aria-labelledby="settings-updates">
    <div className="group-head"><h2 id="settings-updates">Atualizações</h2><p>{update ? `Versão ${update.currentVersion}` : 'Versão deste aplicativo'}</p></div>
    <div className="modular"><div className="block span-12">
      <div className="block-body settings-updates-body">
        <p role="status" aria-live="polite">{status}</p>
        {update?.status === 'downloading' && <div className="settings-update-download">
          <progress aria-label="Download da atualização" max={total || 100} value={total ? Math.min(downloaded, total) : undefined} />
          <span>{percent === null ? `${formatBytes(downloaded)} recebidos` : `${percent}% · ${formatBytes(downloaded)} de ${formatBytes(total!)}`}</span>
        </div>}
        {update?.status === 'disabled' && update.error && <p className="settings-update-detail">{update.error}</p>}
        {(error || readError || (update?.status === 'error' && update.error)) && <p className="form-error" role="alert">{error || readError || update?.error}</p>}
        <div className="settings-update-actions">
          <button className="button" type="button" disabled={unavailable || working || Boolean(action)} onClick={() => void run('check')}>{action === 'check' || update?.status === 'checking' ? <LoaderCircle className="spin" size={16} /> : <RefreshCw size={16} />}Verificar atualizações</button>
          {['ready', 'waiting_for_idle'].includes(update?.status || '') && <button className="button primary" type="button" disabled={Boolean(action)} onClick={() => void run('install')}>{action === 'install' && <LoaderCircle className="spin" size={16} />}Aplicar e reiniciar</button>}
        </div>
      </div>
      {update?.lastCheckedAt && <details className="extra-details settings-extra"><summary>Detalhes da atualização</summary><p className="settings-update-time">Última verificação: <time dateTime={update.lastCheckedAt}>{formatUpdateTime(update.lastCheckedAt)}</time></p></details>}
    </div></div>
  </section>;
}

function updateMessage(update: UpdateStatus) {
  const version = update.availableVersion ? ` ${update.availableVersion}` : '';
  switch (update.status) {
    case 'idle': return 'Nenhuma verificação realizada nesta sessão.';
    case 'disabled': return 'Atualizações indisponíveis nesta instalação.';
    case 'checking': return 'Verificando se há uma nova versão…';
    case 'up_to_date': return 'Você está usando a versão mais recente.';
    case 'available': return `Nova versão${version} disponível. Preparando o download…`;
    case 'downloading': return `Baixando a versão${version}…`;
    case 'ready': return `Versão${version} pronta para aplicar. O aplicativo será reiniciado.`;
    case 'waiting_for_idle': return `Versão${version} pronta. Aguardando as tarefas e os formulários em uso antes de reiniciar.`;
    case 'installing': return `Aplicando a versão${version}… O aplicativo será reiniciado.`;
    case 'error': return 'A atualização não foi concluída. Você pode tentar verificar novamente.';
  }
}

function formatBytes(value: number) {
  if (value < 1024 * 1024) return `${(value / 1024).toLocaleString('pt-BR', { maximumFractionDigits: 0 })} KB`;
  return `${(value / (1024 * 1024)).toLocaleString('pt-BR', { maximumFractionDigits: 1 })} MB`;
}

function formatUpdateTime(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString('pt-BR', { dateStyle: 'short', timeStyle: 'short' });
}

function MarketSettings() {
  const publicData = usePublicData();
  const [settings, setSettings] = useState<MarketDataSettings | null>(null);
  const [loading, setLoading] = useState(desktop);
  const [key, setKey] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const request = useRef(0);
  const pending = useRef(false);
  useEffect(() => {
    if (!desktop) return;
    const current = ++request.current;
    void readPublicCache<MarketDataSettings>('get_market_data_settings')
      .then((result) => { if (current === request.current) setSettings(result); })
      .catch((err) => { if (current === request.current) setError(errorMessage(err)); })
      .finally(() => { if (current === request.current) setLoading(false); });
    return () => { request.current += 1; };
  }, []);
  const changeKey = async (remove: boolean) => {
    if (pending.current || (!remove && !key.trim())) return;
    const current = ++request.current;
    pending.current = true; setBusy(true); setError(''); setNotice('');
    try {
      const result = await call<MarketDataSettings>(remove ? 'clear_brapi_key' : 'save_brapi_key', remove ? undefined : { key: key.trim() });
      if (current !== request.current) return;
      setSettings(result); setKey('');
      publicData.configurationChanged(result);
      setNotice(remove ? result.brapiKeyConfigured ? 'Chave local removida. A chave do ambiente continua configurada.' : 'Chave local removida.' : 'Chave salva neste computador. A consulta automática verificará as cotações da carteira.');
    } catch (err) { if (current === request.current) setError(errorMessage(err)); }
    finally { if (current === request.current) { pending.current = false; setBusy(false); } }
  };
  const locked = !desktop || loading || busy;
  return <section className="group" aria-labelledby="settings-market">
    <div className="group-head"><h2 id="settings-market">Dados de mercado</h2><p>Credencial deste computador</p></div>
    <div className="modular">
      <div className="block span-12">
        <SectionHeading title="Cotações · brapi" level={3} meta={loading ? 'Carregando…' : settings ? settings.brapiKeyConfigured ? settings.brapiKeySource === 'environment' ? 'Chave do ambiente configurada' : 'Chave local configurada' : 'Sem chave configurada' : 'Configuração indisponível'} />
        <form className="block-body settings-market-form" onSubmit={(event) => { event.preventDefault(); void changeKey(false); }}>
          <p className="settings-market-intro"><PublicSourceLink href="https://brapi.dev/dashboard">Obter uma chave na brapi</PublicSourceLink></p>
          <Field label="Chave da brapi" hint="A chave salva não é exibida neste campo."><input type="password" name="brapi-local-key" autoComplete="new-password" spellCheck={false} autoCapitalize="none" value={key} onChange={(event) => setKey(event.target.value)} placeholder={settings?.brapiKeyConfigured ? 'Digite uma nova chave para substituir' : 'Cole sua chave gratuita'} disabled={locked} maxLength={512} /></Field>
          {error && <p className="form-error" role="alert">{error}</p>}
          {notice && <p className="settings-market-notice" role="status">{notice}</p>}
          {!desktop && <p className="settings-market-notice">O cadastro e as consultas estão disponíveis no aplicativo Windows.</p>}
          <div className="settings-market-actions"><button className="button primary" type="submit" disabled={locked || !key.trim()}>{busy ? <LoaderCircle className="spin" size={16} /> : <Save size={16} />}{busy ? 'Salvando…' : 'Salvar chave'}</button><button className="button" type="button" disabled={locked || settings?.brapiKeySource !== 'local'} onClick={() => void changeKey(true)}>Remover chave local</button></div>
        </form>
        <details className="extra-details settings-extra"><summary>Sobre as fontes</summary><div className="settings-market-cvm"><p>A credencial fica neste computador e não entra nos backups da carteira nem na sincronização. A cobertura e os limites das cotações são definidos pela brapi.</p><p>Os informes mensais da CVM dispensam chave. Na tela Mercado, confira ou corrija o vínculo do ticker com o CNPJ do fundo.</p><p>O aplicativo usa os dados salvos primeiro e consulta as fontes automaticamente quando necessário, com limites. Você também pode atualizar manualmente ou solicitar ao assistente.</p><PublicSourceLink href="https://dados.cvm.gov.br/dataset/fii-doc-inf_mensal">Informes FII da CVM</PublicSourceLink><PublicSourceLink href="https://dados.cvm.gov.br/dataset/fiagro-doc-inf_mensal">Informes FIAGRO da CVM</PublicSourceLink></div></details>
      </div>
    </div>
  </section>;
}

function PreferenceRow({ title, description, checked, onChange, disabled, sub }: { title: string; description: string; checked: boolean; onChange: (value: boolean) => void; disabled?: boolean; sub?: boolean }) {
  const id = useId();
  return <li><div className={`settings-row settings-check${sub ? ' settings-sub' : ''}`}>
    <span className="settings-row-text"><span className="settings-row-title" id={`${id}-title`}><Def text={description}>{title}</Def></span><span className="sr-only" id={`${id}-desc`}>{description}</span></span>
    <input type="checkbox" role="switch" checked={checked} onChange={(event) => onChange(event.target.checked)} disabled={disabled} aria-labelledby={`${id}-title`} aria-describedby={`${id}-desc`} />
  </div></li>;
}

function ActionRow({ title, description, children }: { title: string; description: string; children: (describedBy: string) => ReactNode }) {
  const id = useId();
  return <li className="settings-action">
    <span className="settings-row-title"><Def text={description}>{title}</Def></span>
    <span className="sr-only" id={`${id}-desc`}>{description}</span>
    <span className="settings-row-control">{children(`${id}-desc`)}</span>
  </li>;
}

function PathRow({ label, children }: { label: string; children: ReactNode }) {
  return <div><dt>{label}</dt><dd>{children}</dd></div>;
}

function MigrationStatus({ migration }: { migration: Record<string, unknown> }) {
  if (migration.error) return <PathRow label="Importação automática"><p className="form-error" role="alert">A importação automática não foi concluída: {String(migration.error)}. O arquivo original permanece preservado.</p></PathRow>;
  return <>
    <PathRow label="Importação automática"><span className="num">{Number(migration.fundCount) || 0}</span> fundos e <span className="num">{Number(migration.recordCount) || 0}</span> lançamentos importados. Arquivo original preservado.</PathRow>
    {Boolean(migration.backupPath) && <PathRow label="Cópia de segurança"><code>{String(migration.backupPath)}</code></PathRow>}
  </>;
}
