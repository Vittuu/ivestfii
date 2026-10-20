import { useEffect, useRef, useState } from 'react';
import type { MouseEvent, ReactNode } from 'react';
import { call, desktop, errorMessage } from '../lib/bridge';

/**
 * Link para uma fonte pública, aberto no navegador do sistema.
 * `after` fica logo depois do link e antes da mensagem de erro, para que um
 * complemento na mesma linha ("· ref. 15/10/2026") não seja separado dele pelo aviso.
 */
export default function PublicSourceLink({ href, children, className, after }: { href: string; children: ReactNode; className?: string; after?: ReactNode }) {
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);
  const request = useRef(0);
  useEffect(() => {
    setError(''); setBusy(false); pending.current = false;
    return () => { request.current += 1; };
  }, [href]);

  const open = async (event: MouseEvent<HTMLAnchorElement>) => {
    if (!desktop || (event.type === 'auxclick' && event.button !== 1)) return;
    event.preventDefault();
    if (pending.current) return;
    const current = ++request.current;
    pending.current = true; setBusy(true); setError('');
    try { await call('open_public_source', { url: href }); }
    catch (err) { if (current === request.current) setError(`Não foi possível abrir a fonte: ${errorMessage(err)}`); }
    finally { if (current === request.current) { pending.current = false; setBusy(false); } }
  };

  return <><a className={className} href={href} target="_blank" rel="noreferrer" onClick={(event) => void open(event)} onAuxClick={(event) => void open(event)} aria-busy={busy || undefined}>{children}<span className="sr-only"> (abre no navegador)</span></a>{after}{error && <span className="public-source-error" role="alert">{error}</span>}</>;
}
