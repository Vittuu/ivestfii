"""Verify native import, persistence and full backup without opening a window.

Reads the local source files at runtime. No personal values are embedded in the
frontend, the executable, or this verification script. All databases are isolated.
"""
from decimal import Decimal
from hashlib import sha256
import json
from pathlib import Path
import subprocess
import uuid


root = Path(__file__).resolve().parent.parent
binary = root / 'target/debug/ivestfii-desktop.exe'
legacy = root / 'data/fiis_data.json'
positions = sorted((root / 'data').glob('posicao_*.json'))
if not binary.is_file() or not positions:
    raise SystemExit('Compile o aplicativo e mantenha o JSON de posição na pasta data/.')
position = positions[-1]
originals = {path: path.read_bytes() for path in (legacy, position)}
source = json.loads(originals[position])
directory = root / '.local' / f'complete-verification-{uuid.uuid4().hex}'
directory.mkdir(parents=True)


def read_json(path):
    return json.loads(path.read_text(encoding='utf-8'), parse_float=Decimal)


def run_native(destination, history, position_file=None):
    args = [str(binary), '--verify-storage', '--data-dir', str(destination),
            '--legacy-file', str(history)]
    if position_file:
        args.extend(['--position-file', str(position_file)])
    process = subprocess.run(args, capture_output=True, text=True, encoding='utf-8',
                             timeout=30, creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
    if process.returncode:
        raise RuntimeError(process.stderr.strip())
    return json.loads(process.stdout), read_json(destination / 'verified-snapshot.json')


first, snapshot = run_native(directory, legacy, position)
second, repeated = run_native(directory, legacy, position)
assert not first['migration']['alreadyImported'] and second['migration']['alreadyImported']
assert snapshot == repeated, 'A segunda execução alterou ou duplicou os dados.'
broker = snapshot['brokerPosition']
assert broker is not None and first['hasBrokerPosition']
assert broker['sourceHash'] == sha256(originals[position]).hexdigest()
assert broker['referenceDate'] == source['data_referencia']
overview = source['panorama_carteira']
for actual, expected in [
    ('totalWithCash', 'total_investido_mais_conta_investimento'),
    ('cash', 'saldo_conta_investimento_nacional'),
    ('invested', 'total_investido_em_classes'),
]:
    assert Decimal(broker['overview'][actual]) == Decimal(overview[expected])
assert Decimal(broker['overview']['totalWithCash']) == (
    Decimal(broker['overview']['invested']) + Decimal(broker['overview']['cash']))
assert Decimal(broker['listedFunds']['marketValue']) == sum(
    (Decimal(holding['marketValue']) for holding in broker['listedFunds']['holdings']), Decimal(0))
assert Decimal(broker['fixedIncome']['balance']) == Decimal(source['posicao_pos_fixado']['ativo']['saldo'])
assert Decimal(broker['fixedIncome']['netBalance']) == Decimal(source['posicao_pos_fixado']['ativo']['saldo_liquido'])
for original_holding in source['posicao_fundos_listados']['ativos']:
    holding = next(item for item in broker['listedFunds']['holdings'] if item['ticker'] == original_holding['ticker'])
    for actual, expected in [('shares', 'quantidade_cotas'), ('averagePrice', 'preco_medio_exibido'),
                             ('price', 'preco_atual_exibido'), ('marketValue', 'posicao_exibida'),
                             ('unrealizedResult', 'resultado_total_exibido'), ('costBasis', 'custo_total_inferido')]:
        assert Decimal(holding[actual]) == Decimal(str(original_holding[expected]))
exported = directory / 'verified-export.json'
backup = read_json(exported)
assert backup['broker_position'] == broker, 'A exportação perdeu a posição completa.'
restored, restored_snapshot = run_native(directory / 'restored', exported)
assert restored['hasBrokerPosition']
assert restored_snapshot['brokerPosition'] == broker, 'A restauração alterou a posição ou a proveniência.'


def history_values(value):
    return sorted((fund['ticker'], fund['totalCotas'], fund['totalInvested'],
                   fund['dividendsReceived'], sorted((entry['month'], entry['notes'], entry['cotasAdded'],
                   entry['pricePerCota'], entry['dividendPerCota'], entry['dividendTotal'])
                   for entry in fund['entries'])) for fund in value['funds'])


assert history_values(restored_snapshot) == history_values(snapshot)
assert restored_snapshot['totals'] == snapshot['totals']
for path, raw in originals.items():
    assert path.read_bytes() == raw, f'O arquivo de origem foi alterado: {path.name}'
position_backup = directory / 'backups' / f'broker-position-{broker["sourceHash"]}.json'
assert position_backup.is_file(), 'A cópia exata da posição não foi criada.'
assert position_backup.read_bytes() == originals[position]
print(json.dumps({
    'status': 'ok', 'fundCount': len(snapshot['funds']), 'recordCount': snapshot['recordCount'],
    'holdingsCount': len(broker['listedFunds']['holdings']), 'brokerPositionPersisted': True,
    'idempotent': True, 'completeBackupRestored': True, 'sourcesUnchanged': True,
    'positionBackupExact': True,
}))
