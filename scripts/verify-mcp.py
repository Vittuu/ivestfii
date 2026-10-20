"""Smoke-test the actual MCP subprocess against the isolated migrated portfolio."""
import json
from pathlib import Path
import sqlite3
import subprocess
import argparse
from datetime import date, datetime

arguments = argparse.ArgumentParser(description=__doc__)
arguments.add_argument('--desktop', action='store_true', help='Verificar o MCP embutido no executável Tauri.')
arguments.add_argument('--binary', type=Path, help='Executável alternativo; use --desktop para um executável Tauri.')
arguments.add_argument('--refresh-public-data', action='store_true',
                       help='Consultar BCB/EBC reais e conferir cache; não envia dados financeiros nem consome IA.')
options = arguments.parse_args()

root = Path(__file__).resolve().parent.parent
report_path = root / '.local' / 'last-verification.json'
if not report_path.is_file():
    raise SystemExit('Execute antes: python -B scripts/verify-migration.py')
report = json.loads(report_path.read_text(encoding='utf-8'))
directory = Path(report['dataDirectory'])
binary = options.binary.resolve() if options.binary else root / 'target' / 'debug' / ('ivestfii-desktop.exe' if options.desktop else 'ivestfii-mcp.exe')
if not binary.is_file():
    raise SystemExit('Compile antes o executável desktop ou ivestfii-mcp correspondente.')


def counts():
    with sqlite3.connect(directory / 'portfolio.sqlite') as connection:
        return tuple(connection.execute(f'SELECT COUNT(*) FROM {table}').fetchone()[0]
                     for table in ('funds', 'monthly_records', 'cash_pockets', 'sync_outbox'))


before = counts()
source_path = Path(report.get('sourcePath', root / 'examples' / 'portfolio.example.json'))
source_before = source_path.read_bytes() if source_path.is_file() else None
frames = [
    {'jsonrpc': '2.0', 'id': 'init', 'method': 'initialize', 'params': {
        'protocolVersion': '2025-11-25', 'capabilities': {},
        'clientInfo': {'name': 'ivestfii-verification', 'version': '0.1.0'},
    }},
    {'jsonrpc': '2.0', 'method': 'notifications/initialized'},
    {'jsonrpc': '2.0', 'id': 'tools', 'method': 'tools/list'},
    {'jsonrpc': '2.0', 'id': 'portfolio', 'method': 'tools/call', 'params': {
        'name': 'carteira_snapshot', 'arguments': {},
    }},
    {'jsonrpc': '2.0', 'id': 'projection', 'method': 'tools/call', 'params': {
        'name': 'projetar_renda', 'arguments': {'months': 3, 'monthlyCotas': '0', 'reinvest': False},
    }},
    {'jsonrpc': '2.0', 'id': 'prepare', 'method': 'tools/call', 'params': {
        'name': 'preparar_analise_carteira', 'arguments': {},
    }},
    {'jsonrpc': '2.0', 'id': 'invalid', 'method': 'tools/call', 'params': {
        'name': 'projetar_renda', 'arguments': {'months': 0},
    }},
]
if options.refresh_public_data:
    frames.extend([
        {'jsonrpc': '2.0', 'id': 'public', 'method': 'tools/call', 'params': {
            'name': 'preparar_analise_carteira',
            'arguments': {'refreshEconomicData': True, 'refreshNews': True},
        }},
        {'jsonrpc': '2.0', 'id': 'cached', 'method': 'tools/call', 'params': {
            'name': 'preparar_analise_carteira', 'arguments': {},
        }},
    ])
result = subprocess.run(
    [str(binary), *(['--mcp'] if options.desktop else []), '--data-dir', str(directory)],
    input=''.join(json.dumps(frame) + '\n' for frame in frames),
    capture_output=True, text=True, encoding='utf-8', timeout=60,
    creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0),
)
if result.returncode:
    raise RuntimeError(f'O servidor MCP falhou: {result.stderr.strip()}')
responses = [json.loads(line) for line in result.stdout.splitlines()]
if len(responses) != len(frames) - 1:
    raise AssertionError('Quantidade de respostas inválida ou stdout contém logs.')
responses = {response['id']: response for response in responses}
assert responses['init']['result']['protocolVersion'] == '2025-11-25'
assert len(responses['tools']['result']['tools']) == 4
portfolio = responses['portfolio']['result']['structuredContent']
assert len(portfolio['funds']) == report['fundCount']
assert portfolio['recordCount'] == report['recordCount']
assert len(responses['projection']['result']['structuredContent']['points']) == 3
prepared = responses['prepare']['result']['structuredContent']
assert prepared['marketData']['status'] == 'unavailable'
assert prepared['economicData']['refreshRequested'] is False
assert prepared['news']['refreshRequested'] is False
assert prepared['news']['status'] in ('unavailable', 'cached')
assert prepared['dividendData']['refreshRequested'] is False
assert prepared['dividendData']['status'] == 'unavailable'
assert prepared['monthlyIncome']['status'] == 'calculated'
assert len(prepared['monthlyIncome']['report']['months']) == 12
assert all(point['apiAmount'] == '0' for point in prepared['monthlyIncome']['report']['months']), 'Sem cache público o MCP inventou renda de API.'
assert prepared['analysis']['status'] == 'not-generated'
assert responses['invalid']['result']['isError'] is True
assert counts() == before, 'MCP alterou carteira ou fila de sincronização.'
assert source_before is None or source_path.read_bytes() == source_before, 'JSON original alterado.'
public_summary = {}
if options.refresh_public_data:
    public = responses['public']['result']['structuredContent']
    cached = responses['cached']['result']['structuredContent']
    assert public['economicData']['status'] == 'available', public['economicData']
    assert public['news']['status'] == 'available', public['news']
    indicators = public['economicData']['indicators']
    assert {item['id'] for item in indicators} == {'cdi', 'selic', 'ipca'}
    for item in indicators:
        assert item['sourceUrl'].startswith('https://api.bcb.gov.br/dados/serie/bcdata.sgs.')
        reference = date.fromisoformat(item['referenceDate'])
        fetched = datetime.fromisoformat(item['fetchedAt'])
        assert reference <= fetched.date()
    for item in public['news']['items']:
        assert item['url'].startswith('https://agenciabrasil.ebc.com.br/')
        if item['publishedAt']:
            datetime.fromisoformat(item['publishedAt'])
    assert cached['economicData']['status'] == 'cached'
    assert cached['news']['status'] == 'cached'
    assert cached['economicData']['indicators'] == indicators
    assert cached['news']['items'] == public['news']['items']
    assert cached['economicData']['refreshRequested'] is False
    assert cached['news']['refreshRequested'] is False
    assert (directory / 'public-data' / 'economics.json').is_file()
    assert (directory / 'public-data' / 'news.json').is_file()
    public_summary = {'publicSources': 'BCB + Agência Brasil',
                      'economicIndicators': len(indicators), 'headlines': len(public['news']['items']),
                      'cacheRoundTrip': True, 'sourceDatesPresent': True}
print(json.dumps({'status': 'ok', 'tools': 4, 'protocol': '2025-11-25',
                  'host': 'desktop' if options.desktop else 'standalone',
                  'realSubprocess': True, 'financialRowsUnchanged': True,
                  'originalUnchanged': True, 'externalDataNotFabricated': True, **public_summary}))
