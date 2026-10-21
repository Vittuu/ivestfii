"""Verify a cash-schema upgrade on SQLite backups, leaving the source untouched.

Only public cached JSON is copied. No key is read or passed by this script, no
network refresh is requested, and no installed app is started or stopped.
"""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import shutil
import sqlite3
import subprocess
import uuid

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, required=True)
parser.add_argument('--data-dir', type=Path, required=True)
options = parser.parse_args()
binary = options.binary.resolve(strict=True)
source = options.data_dir.resolve(strict=True)
workspace = Path(__file__).resolve().parents[1]
destination = workspace / '.local' / ('cash-upgrade-' + uuid.uuid4().hex)
destination.mkdir(parents=True, exist_ok=False)


def database_state(path):
    with sqlite3.connect(path.as_uri() + '?mode=ro', uri=True) as connection:
        tables = connection.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").fetchall()
        result = {}
        for (table,) in tables:
            quoted = table.replace('"', '""')
            columns = [item[1] for item in connection.execute(f'PRAGMA table_info("{quoted}")')]
            rows = connection.execute(f'SELECT * FROM "{quoted}" ORDER BY rowid').fetchall()
            result[table] = {'columns': columns, 'rows': rows}
        return {'version': connection.execute('PRAGMA user_version').fetchone()[0], 'tables': result}


def digest(value):
    return sha256(json.dumps(value, sort_keys=True, default=str).encode()).hexdigest()


paths = [path for path in source.rglob('*.sqlite') if path.name != 'assistant.sqlite']
before = {str(path.relative_to(source)): database_state(path) for path in paths}
for path in paths:
    target = destination / path.relative_to(source)
    target.parent.mkdir(parents=True, exist_ok=True)
    with sqlite3.connect(path.as_uri() + '?mode=ro', uri=True) as original, sqlite3.connect(target) as backup:
        original.backup(backup)

public_source = source / 'public-data'
if public_source.exists():
    (destination / 'public-data').mkdir()
    for path in public_source.glob('*.json'):
        shutil.copyfile(path, destination / 'public-data' / path.name)

with sqlite3.connect((destination / 'wallets.sqlite').as_uri() + '?mode=ro', uri=True) as catalog:
    wallet_ids = [row[0] for row in catalog.execute('SELECT id FROM wallets ORDER BY id')]
# MCP requires the local assistant store to exist. Use a fresh empty database
# rather than copying any of the user's conversations into this financial test.
with sqlite3.connect(destination / 'assistant.sqlite') as assistant:
    assistant.execute('PRAGMA user_version=0')
frames = [
    {'jsonrpc': '2.0', 'id': 'init', 'method': 'initialize', 'params': {'protocolVersion': '2025-11-25', 'capabilities': {}, 'clientInfo': {'name': 'cash-upgrade-verifier', 'version': '1'}}},
    {'jsonrpc': '2.0', 'method': 'notifications/initialized'},
]
for index, wallet in enumerate(wallet_ids):
    frames.append({'jsonrpc': '2.0', 'id': f'wallet-{index}', 'method': 'tools/call', 'params': {'name': 'preparar_analise_carteira', 'arguments': {'walletId': wallet}}})
reply = subprocess.run([str(binary), '--mcp', '--data-dir', str(destination)],
    input=''.join(json.dumps(frame) + '\n' for frame in frames), text=True, capture_output=True,
    encoding='utf-8', timeout=90, creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))

# The source comparison runs even when the subprocess fails.
assert all(digest(database_state(source / relative)) == digest(state) for relative, state in before.items()), 'O diretório original foi alterado.'
if reply.returncode:
    (destination / 'native-error.txt').write_text(reply.stderr, encoding='utf-8')
assert reply.returncode == 0, 'O verificador nativo não concluiu a atualização da cópia.'
responses = {response['id']: response for response in map(json.loads, reply.stdout.splitlines())}
for index in range(len(wallet_ids)):
    response = responses[f'wallet-{index}']
    assert 'error' not in response and not response['result'].get('isError'), 'A leitura da carteira copiada falhou.'

added = {'total_deposits', 'total_withdrawals', 'balance_date', 'cash_flows'}
for relative, state in before.items():
    target = destination / relative
    upgraded = database_state(target)
    with sqlite3.connect(target.as_uri() + '?mode=ro', uri=True) as connection:
        for table, original in state['tables'].items():
            quoted = table.replace('"', '""')
            columns = ','.join('"' + column.replace('"', '""') + '"' for column in original['columns'])
            rows = connection.execute(f'SELECT {columns} FROM "{quoted}" ORDER BY rowid').fetchall()
            assert rows == original['rows'], f'Uma coluna financeira existente mudou: {relative}/{table}'
        if 'cash_pockets' in state['tables']:
            assert upgraded['version'] == 10
            assert 'performance_history' in upgraded['tables']
            if 'performance_history' not in state['tables']:
                assert connection.execute('SELECT COUNT(*) FROM performance_history').fetchone()[0] == 0, 'A leitura MCP inventou observações mensais.'
            assert added.issubset(upgraded['tables']['cash_pockets']['columns'])
            assert 'purchase_date' in upgraded['tables']['monthly_records']['columns']
            if 'cash_flows' not in state['tables']['cash_pockets']['columns']:
                assert connection.execute('SELECT COUNT(*) FROM cash_pockets WHERE cash_flows IS NOT NULL').fetchone()[0] == 0, 'A atualização inventou datas de movimentos.'
            if 'purchase_date' not in state['tables']['monthly_records']['columns']:
                assert connection.execute('SELECT COUNT(*) FROM monthly_records WHERE purchase_date IS NOT NULL').fetchone()[0] == 0, 'A atualização inventou datas de compras.'
            if not added.intersection(state['tables']['cash_pockets']['columns']):
                assert connection.execute('SELECT COUNT(*) FROM cash_pockets WHERE total_deposits IS NOT NULL OR total_withdrawals IS NOT NULL OR balance_date IS NOT NULL').fetchone()[0] == 0, 'A atualização inventou dados de rendimento.'
        else:
            assert upgraded['version'] == state['version'], 'O catálogo mudou de versão inesperadamente.'

report = {'status': 'ok', 'sourceUnchanged': True, 'existingFinancialColumnsUnchanged': True,
    'legacyCashBasisNotInferred': True, 'legacyPurchaseDatesNotInferred': True, 'walletsVerified': len(wallet_ids), 'schemaVersion': 10,
    'verificationDirectory': str(destination)}
(destination / 'report.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
print(json.dumps(report))
