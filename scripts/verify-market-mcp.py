"""Verify the actual desktop MCP, printing only public coverage and integrity.

Explicit --refresh opts into public-source requests. The key is read by the
native backend from its OS vault; it is never passed to this script.
"""
import argparse
from datetime import datetime
from decimal import Decimal
from hashlib import sha256
import json
from pathlib import Path
import sqlite3
import subprocess
from urllib.parse import parse_qs, urlsplit

KNOWN_PORTFOLIO = {
    'MXRF11': ('97521225000125', 'FII'),
    'VGHF11': ('36771692000119', 'FII'),
    'VGIA11': ('41081088000109', 'FIAGRO'),
    'XPML11': ('28757546000100', 'FII'),
}

arguments = argparse.ArgumentParser(description=__doc__)
arguments.add_argument('--binary', type=Path, required=True)
arguments.add_argument('--data-dir', type=Path, required=True)
arguments.add_argument('--wallet-id')
arguments.add_argument('--refresh', action='store_true')
options = arguments.parse_args()
directory = options.data_dir.resolve(strict=True)
binary = options.binary.resolve(strict=True)


def fingerprints():
    result = {}
    for path in directory.rglob('*.sqlite'):
        if path.name == 'assistant.sqlite':
            continue
        with sqlite3.connect(path.as_uri() + '?mode=ro', uri=True) as connection:
            tables = connection.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").fetchall()
            data = []
            for (table,) in tables:
                quoted = table.replace('"', '""')
                data.append((table, connection.execute(f'SELECT * FROM "{quoted}" ORDER BY rowid').fetchall()))
            encoded = json.dumps(data, sort_keys=True, default=lambda value: value.hex() if isinstance(value, bytes) else str(value)).encode()
            result[str(path.relative_to(directory))] = sha256(encoded).hexdigest()
    return result


before = fingerprints()
scope = {'walletId': options.wallet_id} if options.wallet_id else {}
frames = [
    {'jsonrpc': '2.0', 'id': 'init', 'method': 'initialize', 'params': {'protocolVersion': '2025-11-25', 'capabilities': {}, 'clientInfo': {'name': 'market-coverage-verification', 'version': '1'}}},
    {'jsonrpc': '2.0', 'method': 'notifications/initialized'},
    {'jsonrpc': '2.0', 'id': 'tools', 'method': 'tools/list'},
    {'jsonrpc': '2.0', 'id': 'before', 'method': 'tools/call', 'params': {'name': 'preparar_analise_carteira', 'arguments': scope}},
]
if options.refresh:
    frames.append({'jsonrpc': '2.0', 'id': 'refresh', 'method': 'tools/call', 'params': {'name': 'preparar_analise_carteira', 'arguments': {**scope, 'refreshQuotes': True, 'refreshCvmData': True}}})
frames.append({'jsonrpc': '2.0', 'id': 'after', 'method': 'tools/call', 'params': {'name': 'preparar_analise_carteira', 'arguments': scope}})
reply = subprocess.run([str(binary), '--mcp', '--data-dir', str(directory)], input=''.join(json.dumps(frame) + '\n' for frame in frames), capture_output=True, text=True, encoding='utf-8', timeout=100, creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
# Check integrity before coverage assertions so even an incomplete source result
# cannot skip the comparison of the financial databases and wallet catalog.
assert fingerprints() == before, 'Os bancos financeiros ou catálogo foram alterados.'
if reply.returncode:
    raise RuntimeError('O subprocesso MCP não concluiu a verificação.')
responses = [json.loads(line) for line in reply.stdout.splitlines()]
assert len(responses) == len(frames) - 1, 'stdout MCP contém resposta inválida ou logs.'
responses = {response['id']: response for response in responses}
tools = responses['tools']['result']['tools']
assert len(tools) == 5
analysis = next(tool for tool in tools if tool['name'] == 'preparar_analise_carteira')
for flag in ('refreshQuotes', 'refreshCvmData'):
    assert analysis['inputSchema']['properties'][flag]['type'] == 'boolean'


def content(identifier):
    result = responses[identifier]['result']
    assert not result.get('isError'), 'A ferramenta MCP retornou erro.'
    return result['structuredContent']


initial = content('before')
final = content('after')
assert not initial['marketData']['refreshRequested'] and not initial['cvmData']['refreshRequested']
assert not final['marketData']['refreshRequested'] and not final['cvmData']['refreshRequested']
observed = content('refresh') if options.refresh else final
assert not observed['marketData']['errors'], 'Falha de consulta ou cache de cotações.'
assert not observed['cvmData']['errors'], 'Falha de consulta ou cache CVM.'
quotes = observed['marketData']['items']
reports = observed['cvmData']['items']
portfolio = observed['portfolio']
portfolio_symbols = {
    item['ticker'].strip().upper()
    for item in portfolio['funds'] + (portfolio.get('brokerPosition') or {}).get('listedFunds', {}).get('holdings', [])
}
for items in (quotes, reports):
    symbols = [item['symbol'] for item in items]
    assert len(symbols) == len(set(symbols)), 'A fonte retornou símbolos duplicados.'
    assert set(symbols) == portfolio_symbols, 'A cobertura não corresponde aos ativos da carteira selecionada.'
known_portfolio = portfolio_symbols == set(KNOWN_PORTFOLIO)
if options.refresh and known_portfolio:
    assert len(quotes) == len(reports) == 4, 'A carteira conhecida exige quatro cotações e quatro informes CVM.'
    assert observed['marketData']['refreshRequested'] and observed['cvmData']['refreshRequested']
for item in quotes:
    observation = item.get('observation')
    assert observation and Decimal(observation['price']) > 0, 'Um ativo não possui cotação validada.'
    assert observation['currency'] == 'BRL'
    source = urlsplit(observation['sourceUrl'])
    assert source.scheme == 'https' and source.netloc == 'brapi.dev'
    assert source.path == '/api/v2/stocks/quote' and parse_qs(source.query) == {'symbols': [item['symbol']]}
    assert observation['source'] == 'brapi' and observation['symbol'] == item['symbol']
    assert item['status'] in ('available', 'cached') and not item.get('error')
    if options.refresh:
        assert item['status'] == 'available', 'A atualização não produziu uma cotação validada para cada ativo.'
    datetime.fromisoformat(observation['fetchedAt'])
    if observation['quotedAt']:
        datetime.fromisoformat(observation['quotedAt'])
for item in reports:
    report = item.get('report')
    if options.refresh and known_portfolio:
        assert report and item['status'] in ('available', 'cached') and not item.get('error'), 'Um dos quatro ativos não possui informe CVM validado.'
    if report:
        assert item['cnpj'] == report['cnpj']
        fund_type = report['fundType']
        assert fund_type in ('FII', 'FIAGRO'), 'O informe não identifica sua classe de fundo.'
        reference = datetime.fromisoformat(report['referenceDate'])
        assert reference.day == 1, 'A competência mensal não corresponde ao primeiro dia do mês.'
        source = urlsplit(report['sourceUrl'])
        assert source.scheme == 'https' and source.netloc == 'dados.cvm.gov.br' and not source.query and not source.fragment
        if fund_type == 'FII':
            assert source.path == f'/dados/FII/DOC/INF_MENSAL/DADOS/inf_mensal_fii_{reference.year}.zip'
            assert report['source'] == 'CVM · Informe Mensal Estruturado de FII'
        else:
            assert source.path == f'/dados/FIAGRO/DOC/INF_MENSAL/DADOS/inf_mensal_fiagro_{reference:%Y%m}.zip'
            assert report['source'] == 'CVM · Informe Mensal de FIAGRO'
        if known_portfolio:
            assert (report['cnpj'], fund_type) == KNOWN_PORTFOLIO[item['symbol']], 'Vínculo CNPJ ou classe de fundo incorreto.'
        if report['filedAt']:
            assert datetime.fromisoformat(report['filedAt']).date() >= reference.date()
        datetime.fromisoformat(report['fetchedAt'])
for section, field in [('marketData', 'observation'), ('cvmData', 'report')]:
    expected = {item['symbol']: item[field] for item in observed[section]['items']}
    actual = {item['symbol']: item[field] for item in final[section]['items']}
    assert expected == actual, 'Leitura do cache alterou observações ou datas.'
assert fingerprints() == before, 'Os bancos financeiros ou catálogo foram alterados.'
summary = {'status': 'ok', 'tools': 5, 'realDesktopMcp': True, 'liveRefresh': options.refresh, 'knownPortfolioCoverage': known_portfolio, 'credentialConfigured': observed['marketData']['credentialConfigured'], 'quotes': [{'symbol': item['symbol'], 'status': item['status'], 'quotedAt': item['observation']['quotedAt'], 'fetchedAt': item['observation']['fetchedAt']} for item in quotes], 'cvm': [{'symbol': item['symbol'], 'status': item['status'], **{key: item['report'][key] if item['report'] else None for key in ('cnpj', 'fundType', 'referenceDate', 'filedAt', 'fetchedAt', 'sourceUrl')}} for item in reports], 'cacheRoundTrip': True, 'financialDataUnchanged': True}
root = Path(__file__).resolve().parent.parent
(root / '.local').mkdir(exist_ok=True)
(root / '.local/last-market-mcp-verification.json').write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding='utf-8')
print(json.dumps(summary, ensure_ascii=False))
