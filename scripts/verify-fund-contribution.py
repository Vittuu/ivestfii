"""Verify whole-share contribution scenarios using an isolated synthetic desktop MCP.

Requires a desktop executable with fundAllocations support. This script never
builds, starts the GUI, reads the user's wallet or contacts an investment API.
Detailed evidence stays under the ignored .local directory.
"""

import argparse
from copy import deepcopy
from datetime import datetime, timedelta, timezone
from decimal import Decimal, ROUND_FLOOR, ROUND_HALF_UP, localcontext
from hashlib import sha256
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import uuid


PROTOCOL = '2025-11-25'
TOOLS = {'carteira_snapshot', 'projetar_renda', 'recuperar_contexto',
         'preparar_analise_carteira', 'simular_aporte'}
FUNDS = {'TEST11': {'shares': '10', 'cost': '10', 'price': '9.46', 'budget': '170'},
         'ALT11': {'shares': '3', 'cost': '20', 'price': '20', 'budget': '130'}}
CENT = Decimal('0.01')


def check(condition, message):
    if not condition:
        raise AssertionError(message)


def fingerprint(value):
    return sha256(value).hexdigest()


def decimal(value):
    check(isinstance(value, str), f'Expected decimal text, received {value!r}.')
    result = Decimal(value)
    check(result.is_finite(), 'Non-finite decimal in MCP output.')
    return result


def percent(value, total):
    with localcontext() as context:
        context.prec = 90
        return (value / total * 100).quantize(CENT, rounding=ROUND_HALF_UP)


def timestamp(value):
    return value.astimezone(timezone.utc).isoformat(timespec='seconds').replace('+00:00', 'Z')


def database_state(path):
    with sqlite3.connect(path.as_uri() + '?mode=ro', uri=True) as connection:
        check(connection.execute('PRAGMA quick_check').fetchone()[0] == 'ok',
              f'SQLite integrity failure in {path.name}.')
        schemas = connection.execute(
            "SELECT name, sql FROM sqlite_master WHERE type='table' "
            "AND name NOT LIKE 'sqlite_%' ORDER BY name").fetchall()
        tables = {}
        for name, schema in schemas:
            identifier = '"' + name.replace('"', '""') + '"'
            rows = [tuple({'blob': value.hex()} if isinstance(value, bytes) else value
                          for value in row)
                    for row in connection.execute(f'SELECT * FROM {identifier}')]
            tables[name] = {'schema': schema, 'rows': sorted(rows, key=repr)}
        return {'schemaVersion': connection.execute('PRAGMA user_version').fetchone()[0],
                'tables': tables}


def state(directory):
    databases = {str(path.relative_to(directory)): database_state(path)
                 for path in sorted(directory.rglob('*.sqlite'))}
    files = {}
    for path in sorted(directory.rglob('*')):
        if (not path.is_file() or path.suffix == '.sqlite'
                or path.name.endswith(('-wal', '-shm', '-journal'))):
            continue
        files[str(path.relative_to(directory))] = fingerprint(path.read_bytes())
    return {'databases': databases, 'files': files}


def state_hash(value):
    return fingerprint(json.dumps(value, sort_keys=True, ensure_ascii=False).encode('utf-8'))


def environment():
    result = os.environ.copy()
    for key in ('IVESTFII_DATA_DIR', 'OPENAI_API_KEY', 'ANTHROPIC_API_KEY',
                'ANTHROPIC_AUTH_TOKEN', 'GOOGLE_API_KEY', 'GEMINI_API_KEY'):
        result.pop(key, None)
    # Accidental HTTP dependencies must fail locally instead of using an API.
    for key in ('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'http_proxy',
                'https_proxy', 'all_proxy'):
        result[key] = 'http://127.0.0.1:9'
    result['NO_PROXY'] = result['no_proxy'] = ''
    return result


def run_process(binary, directory, work, label, arguments, input_text=None):
    result = subprocess.run(
        [str(binary), *arguments], input=input_text, capture_output=True,
        text=True, encoding='utf-8', timeout=60, env=environment(), cwd=work,
        creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
    (work / f'{label}.stdout.txt').write_text(result.stdout, encoding='utf-8')
    (work / f'{label}.stderr.txt').write_text(result.stderr, encoding='utf-8')
    check(result.returncode == 0,
          f'{label} exited with {result.returncode}: {result.stderr.strip()[:2000]}')
    return result.stdout


def call(request_id, name, arguments):
    return {'jsonrpc': '2.0', 'id': request_id, 'method': 'tools/call',
            'params': {'name': name, 'arguments': arguments}}


def mcp(binary, directory, work, label, requests):
    frames = [
        {'jsonrpc': '2.0', 'id': 'init', 'method': 'initialize', 'params': {
            'protocolVersion': PROTOCOL, 'capabilities': {},
            'clientInfo': {'name': 'ivestfii-synthetic-fund-contribution', 'version': '1'}}},
        {'jsonrpc': '2.0', 'method': 'notifications/initialized'},
        {'jsonrpc': '2.0', 'id': 'tools', 'method': 'tools/list'},
        *requests]
    output = run_process(binary, directory, work, label,
                         ['--mcp', '--data-dir', str(directory)],
                         ''.join(json.dumps(frame) + '\n' for frame in frames))
    responses = [json.loads(line) for line in output.splitlines()]
    expected = {frame['id'] for frame in frames if 'id' in frame}
    check(len(responses) == len(expected), 'Unexpected responses or logs on MCP stdout.')
    check({response.get('id') for response in responses} == expected,
          'MCP response IDs differ from the submitted frames.')
    check(all(response.get('jsonrpc') == '2.0' and 'error' not in response
              for response in responses), 'MCP returned a JSON-RPC error.')
    responses = {response['id']: response['result'] for response in responses}
    check(responses['init']['protocolVersion'] == PROTOCOL, 'MCP protocol mismatch.')
    tools = responses['tools']['tools']
    check(len(tools) == 5 and {tool['name'] for tool in tools} == TOOLS,
          'The desktop must expose exactly the five authorized tools.')
    check(all(tool.get('annotations', {}).get('readOnlyHint') is True for tool in tools),
          'A tool lost its read-only annotation.')
    simulator = next(tool for tool in tools if tool['name'] == 'simular_aporte')
    check('fundAllocations' in simulator['inputSchema']['properties'],
          'This desktop does not support fundAllocations; build the updated version.')
    return responses


def content(result):
    check(result.get('isError') is not True, f'Unexpected tool failure: {result!r}')
    value = result.get('structuredContent')
    check(isinstance(value, dict), 'Tool result lacks structuredContent.')
    return value


def check_read_only(directory, expected, source, source_bytes, quote_fixture, quote_bytes):
    actual = state(directory)
    check(actual == expected, 'MCP changed stored rows, outbox, exports or cache files.')
    check(source.read_bytes() == source_bytes, 'Synthetic legacy source changed.')
    check(quote_fixture.read_bytes() == quote_bytes, 'Original synthetic quote fixture changed.')
    return state_hash(actual)


def verify_fresh(payload, quotes, identifiers):
    check(payload['status'] == 'simulation' and payload['kind'] == 'contribution-allocation',
          'Unexpected simulator wrapper.')
    check(payload['recommendationGenerated'] is False and payload['refreshRequested'] is False,
          'The simulator generated advice or requested an API refresh.')
    check(payload['profileCompleteness']['canPersonalize'] is False,
          'The synthetic missing profile was inferred from its funds.')
    check(payload['quotePolicy']['noNetworkRequests'] is True, 'Simulator declared network I/O.')
    check(payload['quotePolicy']['cacheMaxAgeSeconds'] == 1800
          and payload['quotePolicy']['marketMaxAgeDays'] == 7, 'Unexpected quote eligibility policy.')
    simulation = payload['fundSimulation']
    check(simulation['status'] == 'calculated' and simulation['fiiBasisComplete'] is True,
          'Complete synthetic holdings were not usable.')
    rows = {row['symbol']: row for row in simulation['rows']}
    check(len(simulation['rows']) == 2 and set(rows) == set(FUNDS), 'Fund scope changed.')
    expected = {}
    for symbol, fixture in FUNDS.items():
        price, shares, budget = map(Decimal, (fixture['price'], fixture['shares'], fixture['budget']))
        quantity = (budget / price).to_integral_value(rounding=ROUND_FLOOR)
        spent = quantity * price
        expected[symbol] = {'price': price, 'shares': shares, 'budget': budget,
                            'quantity': quantity, 'spent': spent, 'remainder': budget - spent,
                            'current': shares * price, 'after': (shares + quantity) * price}
    total_before = sum(item['current'] for item in expected.values())
    spent = sum(item['spent'] for item in expected.values())
    remainder = sum(item['remainder'] for item in expected.values())
    total_fii_after = total_before + spent
    total_after = total_before + Decimal('500')
    check(total_before == Decimal('154.60') and spent == Decimal('280.82')
          and remainder == Decimal('19.18') and total_after == Decimal('654.60'),
          'Independent synthetic arithmetic fixture is inconsistent.')
    check(decimal(simulation['budget']) == Decimal('300')
          and decimal(simulation['spentAmount']) == spent
          and decimal(simulation['remainder']) == remainder, 'Aggregate budget was not conserved.')
    for symbol, row in rows.items():
        item = expected[symbol]
        check(row['fundId'] == identifiers[symbol], 'Fund ID was replaced or crossed wallet scope.')
        check(row['quoteStatus'] == 'usable' and row['quote']['status'] == 'cached',
              'A saved observation was not preserved as a cached quote.')
        check(row['quote']['observation'] == quotes['observations'][symbol],
              'The original quote, dates or source were modified.')
        for key, value in {'budget': item['budget'], 'price': item['price'],
                           'currentShares': item['shares'], 'sharesToBuy': item['quantity'],
                           'afterShares': item['shares'] + item['quantity'],
                           'spentAmount': item['spent'], 'remainder': item['remainder'],
                           'currentValue': item['current'], 'afterValue': item['after']}.items():
            check(decimal(row[key]) == value, f'{symbol}: wrong {key}.')
        check(decimal(row['sharesToBuy']) == decimal(row['sharesToBuy']).to_integral_value(),
              'The scenario bought fractional shares.')
        check(item['spent'] <= item['budget'] < item['spent'] + item['price'],
              'Whole-share floor does not fit the individual budget.')
        for field, numerator, denominator in (
                ('beforeFiiPercent', item['current'], total_before),
                ('afterFiiPercent', item['after'], total_fii_after),
                ('beforePortfolioPercent', item['current'], total_before),
                ('afterPortfolioPercent', item['after'], total_after)):
            check(decimal(row[field]) == percent(numerator, denominator),
                  f'{symbol}: wrong weight or denominator for {field}.')
    portfolio = simulation['portfolioAllocation']
    check(decimal(portfolio['totalBefore']) == total_before
          and decimal(portfolio['totalAfter']) == total_after
          and decimal(portfolio['unallocatedCash']) == remainder, 'Portfolio totals lost money.')
    classes = {row['kind']: row for row in portfolio['rows']}
    check(len(portfolio['rows']) == 4
          and set(classes) == {'fiis', 'fixedIncome', 'cashPockets', 'unallocatedCash'},
          'The leftover must remain a fourth component.')
    for kind, before, contribution in (
            ('fiis', total_before, spent), ('fixedIncome', Decimal(0), Decimal(100)),
            ('cashPockets', Decimal(0), Decimal(100)),
            ('unallocatedCash', Decimal(0), remainder)):
        row = classes[kind]
        check(decimal(row['currentValue']) == before
              and decimal(row['contribution']) == contribution
              and decimal(row['afterValue']) == before + contribution,
              f'{kind}: class totals recounted or redistributed the leftover.')
        check(decimal(row['beforePercent']) == percent(before, total_before)
              and decimal(row['afterPercent']) == percent(before + contribution, total_after),
              f'{kind}: wrong portfolio allocation weight.')
    check(sum(decimal(row['afterValue']) for row in portfolio['rows']) == total_after,
          'The four components do not conserve the full budget.')
    reserved = {row['kind']: row for row in payload['simulation']['rows']}
    check(decimal(reserved['fiis']['contribution']) == Decimal('300')
          and decimal(reserved['fiis']['afterValue']) == total_before + Decimal('300'),
          'Legacy class reservations were changed into whole-share spending.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True, help='Updated desktop executable.')
    options = parser.parse_args()
    binary = options.binary.resolve()
    check(binary.is_file(), 'The required desktop executable does not exist.')
    root = Path(__file__).resolve().parent.parent
    work = root / '.local' / f'fund-contribution-{uuid.uuid4().hex}'
    directory = work / 'data'
    directory.mkdir(parents=True)
    report = {'status': 'running', 'syntheticOnly': True, 'binary': str(binary),
              'dataDirectory': str(directory), 'network': 'fixed local caches; HTTP proxy guard'}
    try:
        source = work / 'portfolio.synthetic.json'
        legacy = {'fiis': [{'ticker': symbol, 'name': f'Fundo fictício {symbol}',
                           'sector': 'Tijolo fictício' if symbol == 'TEST11' else 'Papel fictício',
                           'entries': [{'month': '2020-01', 'purchase_date': '2020-01-15',
                                        'cotas_added': fixture['shares'],
                                        'price_per_cota': fixture['cost'],
                                        'dividend_per_cota': '0', 'dividend_total': '0',
                                        'notes': 'Fixture sintética; não representa investimento real.'}]}
                          for symbol, fixture in FUNDS.items()]}
        source_bytes = json.dumps(legacy, ensure_ascii=False, indent=2).encode('utf-8')
        source.write_bytes(source_bytes)
        first = json.loads(run_process(binary, directory, work, 'storage-initial',
                                      ['--verify-storage', '--data-dir', str(directory),
                                       '--legacy-file', str(source)]))
        check(first['status'] == 'ok' and first['fundCount'] == 2 and first['recordCount'] == 2,
              'Synthetic migration failed.')
        check(Path(first['migration']['backupPath']).read_bytes() == source_bytes,
              'Synthetic legacy backup is not byte-exact.')
        portfolio_initial = database_state(directory / 'portfolio.sqlite')
        mcp(binary, directory, work, 'catalog-initial', [])
        check(database_state(directory / 'portfolio.sqlite') == portfolio_initial,
              'MCP catalog initialization changed financial rows or the outbox.')
        with sqlite3.connect((directory / 'portfolio.sqlite').as_uri() + '?mode=ro', uri=True) as connection:
            identifiers = {ticker: fund_id for fund_id, ticker in connection.execute('SELECT id,ticker FROM funds')}
        with sqlite3.connect((directory / 'wallets.sqlite').as_uri() + '?mode=ro', uri=True) as connection:
            wallet_id = connection.execute('SELECT id FROM wallets WHERE is_default=1').fetchone()[0]
        now = datetime.now(timezone.utc).replace(microsecond=0)
        quotes = {'version': 1, 'observations': {
            symbol: {'symbol': symbol, 'price': fixture['price'], 'currency': 'BRL',
                     'quotedAt': timestamp(now - timedelta(seconds=20)),
                     'fetchedAt': timestamp(now - timedelta(seconds=10)), 'source': 'brapi',
                     'sourceUrl': f'https://brapi.dev/api/v2/stocks/quote?symbols={symbol}',
                     'delayMinutes': 30}
            for symbol, fixture in FUNDS.items()},
            'lastAttemptAt': timestamp(now - timedelta(seconds=10)), 'retryAfter': None}
        quote_bytes = json.dumps(quotes, ensure_ascii=False, indent=2).encode('utf-8')
        quote_fixture = work / 'quotes.synthetic-original.json'
        quote_fixture.write_bytes(quote_bytes)
        cache = directory / 'public-data' / 'quotes.json'
        cache.parent.mkdir()
        cache.write_bytes(quote_bytes)
        arguments = {'walletId': wallet_id, 'amount': '500.00',
                     'allocations': {'fiis': '300.00', 'fixedIncome': '100.00', 'cashPockets': '100.00'},
                     'fundAllocations': [{'fundId': identifiers[symbol], 'amount': fixture['budget']}
                                         for symbol, fixture in FUNDS.items()]}
        reserved_arguments = deepcopy(arguments)
        reserved_arguments.pop('fundAllocations')
        injected = deepcopy(arguments)
        injected['fundAllocations'][0]['price'] = '0.01'
        before = state(directory)
        fresh = mcp(binary, directory, work, 'mcp-fresh', [
            call('snapshot-before', 'carteira_snapshot', {}),
            call('detail', 'simular_aporte', arguments),
            call('repeat', 'simular_aporte', arguments),
            call('reserved', 'simular_aporte', reserved_arguments),
            call('injected-price', 'simular_aporte', injected),
            call('snapshot-after', 'carteira_snapshot', {})])
        detail = content(fresh['detail'])
        verify_fresh(detail, quotes, identifiers)
        check(detail == content(fresh['repeat']), 'Identical repeated simulations changed their result.')
        check('fundSimulation' not in content(fresh['reserved']), 'The optional mode changed old requests.')
        check(fresh['injected-price'].get('isError') is True,
              'The backend accepted a model-supplied price.')
        check(content(fresh['snapshot-before']) == content(fresh['snapshot-after']),
              'The read-only scenario changed the application snapshot.')
        fresh_state_hash = check_read_only(directory, before, source, source_bytes,
                                           quote_fixture, quote_bytes)
        stale_quotes = deepcopy(quotes)
        stale_quotes['observations']['TEST11']['quotedAt'] = timestamp(now - timedelta(minutes=90))
        stale_quotes['observations']['TEST11']['fetchedAt'] = timestamp(now - timedelta(hours=1))
        cache.write_text(json.dumps(stale_quotes, ensure_ascii=False, indent=2), encoding='utf-8')
        stale_before = state(directory)
        stale = mcp(binary, directory, work, 'mcp-stale', [
            call('detail', 'simular_aporte', arguments), call('repeat', 'simular_aporte', arguments)])
        pending = content(stale['detail'])
        check(pending == content(stale['repeat']), 'Repeated stale simulations changed their result.')
        simulation = pending['fundSimulation']
        row = next(row for row in simulation['rows'] if row['symbol'] == 'TEST11')
        check(row['quoteStatus'] == 'stale' and row['quote']['observation'] == stale_quotes['observations']['TEST11'],
              'A recent check replaced the stale original observation.')
        for field in ('price', 'sharesToBuy', 'spentAmount', 'remainder', 'afterShares'):
            check(row[field] is None, f'Stale {field} became zero or a fabricated value.')
        check(simulation['status'] == 'incomplete' and simulation['fiiBasisComplete'] is False
              and simulation['spentAmount'] is None and simulation['remainder'] is None,
              'Stale input produced complete aggregate spending or leftover.')
        allocation = simulation['portfolioAllocation']
        check(allocation['totalBefore'] is None and allocation['totalAfter'] is None
              and allocation['unallocatedCash'] is None, 'Incomplete bases were totaled or normalized.')
        for fund in simulation['rows']:
            for field in ('beforeFiiPercent', 'afterFiiPercent', 'beforePortfolioPercent', 'afterPortfolioPercent'):
                check(fund[field] is None, 'A partial portfolio was normalized by its known funds.')
        for item in allocation['rows']:
            check(item['beforePercent'] is None and item['afterPercent'] is None,
                  'A partial class allocation received complete weights.')
        stale_state_hash = check_read_only(directory, stale_before, source, source_bytes,
                                           quote_fixture, quote_bytes)
        check(stale_before['databases'] == before['databases'], 'Changing only the fixture cache changed stored rows.')
        second = json.loads(run_process(binary, directory, work, 'storage-final',
                                       ['--verify-storage', '--data-dir', str(directory),
                                        '--legacy-file', str(source)]))
        check(second['migration']['alreadyImported'] is True, 'The legacy fixture was imported again.')
        check_read_only(directory, stale_before, source, source_bytes, quote_fixture, quote_bytes)
        report.update({'status': 'ok', 'tools': 5, 'realDesktopSubprocess': True,
                       'freshStateHash': fresh_state_hash, 'staleStateHash': stale_state_hash,
                       'sourceSha256': fingerprint(source_bytes), 'quoteFixtureSha256': fingerprint(quote_bytes),
                       'financialRowsUnchanged': True, 'outboxUnchanged': True, 'exportsUnchanged': True,
                       'cacheReadsUnchanged': True, 'integerShares': {'TEST11': 17, 'ALT11': 6},
                       'spentAmount': '280.82', 'remainder': '19.18', 'totalAfter': '654.60',
                       'weightsIndependentlyVerified': True, 'staleQuotesStayUnknown': True,
                       'modelSuppliedPriceRejected': True, 'repeatStable': True,
                       'freshResult': detail, 'staleResult': pending})
    except Exception as error:
        report.update({'status': 'failed', 'error': f'{type(error).__name__}: {error}'})
        raise
    finally:
        (work / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps({'status': 'ok', 'tools': 5, 'syntheticOnly': True,
                      'integerShares': True, 'budgetConserved': True,
                      'financialRowsUnchanged': True, 'staleQuotesStayUnknown': True,
                      'reportPath': str(work / 'report.json')}))


if __name__ == '__main__':
    main()
