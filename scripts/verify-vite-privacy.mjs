/** Exercise the real Vite configuration using only an isolated synthetic root. */
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';

const scriptPath = fileURLToPath(import.meta.url);
const configFile = path.resolve(path.dirname(scriptPath), '../vite.config.ts');

export async function verifyVitePrivacy() {
  // Windows runners can expose TEMP through an 8.3 alias (RUNNER~1).
  // Vite rejects paths containing '~'; resolve the existing physical directory
  // before creating the fixture so the public control remains a valid request.
  const temporaryParent = await fs.realpath(path.resolve(os.tmpdir()));
  const root = await fs.mkdtemp(path.join(temporaryParent, 'ivestfii-vite-privacy-'));
  const marker = 'SYNTHETIC_PRIVATE_FIXTURE_MUST_NEVER_BE_SERVED';
  const paths = ['/data/fiis_data.json', '/.local/last-verification.json'];
  const results = [];
  let server;
  try {
    await fs.writeFile(path.join(root, 'index.html'), '<!doctype html><title>Synthetic SPA fallback</title>');
    await fs.writeFile(path.join(root, 'public-control.json'), '{"syntheticPublicControl":true}');
    server = await createServer({
      configFile, root, logLevel: 'silent', clearScreen: false,
      server: { port: 0, strictPort: false, host: '127.0.0.1', open: false },
    });
    await server.listen();
    const address = server.httpServer.address();
    assert.ok(address && typeof address === 'object');
    const base = `http://127.0.0.1:${address.port}`;
    const control = await fetch(base + '/public-control.json', { headers: { Accept: 'application/json' } });
    assert.equal(control.status, 200, `The fixture root must be served normally outside private paths (${root}): ${control.status === 200 ? '' : await control.text()}`);
    assert.deepEqual(await control.json(), { syntheticPublicControl: true });

    // JSON requests do not trigger Vite's HTML fallback when the file is absent.
    for (const requestPath of paths) {
      const response = await fetch(base + requestPath, { headers: { Accept: 'application/json' } });
      assert.equal(response.status, 404, `Missing synthetic file ${requestPath}`);
      await response.body?.cancel();
      results.push({ path: requestPath, state: 'absent', status: response.status });
    }

    for (const requestPath of paths) {
      const fixturePath = path.join(root, requestPath.slice(1));
      await fs.mkdir(path.dirname(fixturePath), { recursive: true });
      await fs.writeFile(fixturePath, JSON.stringify({ syntheticPrivateValue: marker }));
      for (const [route, accept] of [
        [requestPath, 'application/json'],
        [requestPath + '?download=1', 'text/html'],
        [requestPath + '?raw', '*/*'],
        ['/@fs/' + fixturePath.replaceAll('\\', '/'), 'application/json'],
      ]) {
        const response = await fetch(base + route, { headers: { Accept: accept } });
        const body = await response.text();
        assert.equal(response.status, 403, `Existing synthetic private file ${route}`);
        assert.equal(body.includes(marker), false, 'Private fixture contents must never be returned');
        results.push({ path: requestPath, state: 'present', accept, status: response.status });
      }
    }
    return results;
  } finally {
    await server?.close();
    assert.equal(path.dirname(root), temporaryParent, 'Cleanup must stay inside the temporary parent');
    assert.ok(path.basename(root).startsWith('ivestfii-vite-privacy-'));
    await fs.rm(root, { recursive: true, force: true });
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === scriptPath) {
  const results = await verifyVitePrivacy();
  console.log(JSON.stringify({ status: 'PASS', syntheticDataOnly: true, requests: results.length }));
}
