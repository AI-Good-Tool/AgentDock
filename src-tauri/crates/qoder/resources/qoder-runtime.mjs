// Load only the installed Qoder runtime's lazy WASM implementation, never its CLI entrypoint.
// No official code or credentials are included in this project or distributable.
import { readFile, mkdir, writeFile, rename } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { homedir } from 'node:os';
import { pathToFileURL } from 'node:url';

async function load(edition) {
  const app = edition === 'cn' ? 'Qoder CN' : 'Qoder';
  const sdk = edition === 'cn' ? 'qoder-cn-agent-sdk' : 'qoder-agent-sdk';
  // The host passes the installed SDK path (macOS app bundle or Windows install dir).
  const source =
    process.env.AGENTDOCK_QODER_SOURCE ||
    `/Applications/${app}.app/Contents/Resources/app.asar.unpacked/node_modules/@qoder-ai/${sdk}/dist/_worker/qoder-worker-runtime.obf.mjs`;
  let code = await readFile(source, 'utf8');
  // The bundle ends by starting the CLI; cut it off there. Obfuscated names
  // change per release, so match the shape rather than the names:
  //   app <=0.4.4  ok(),Zir(async()=>...
  //   app 0.4.8    fx(),vsr(async()=>...
  //   app 0.4.11   Hm(),"sites"===process.argv[2]?...:(LvA(async A=>...
  const tail = code.length - 2000;
  const entry = [...code.slice(tail).matchAll(/\w+\(\),(?:"sites"===process\.argv\[2\]\?|\w+\(async\b)/g)].pop();
  if (!entry) throw new Error('当前 Qoder 运行时版本不受支持，请更新适配器');
  const at = tail + entry.index;
  const loaderMatch = [...code.matchAll(/async function (\w+)\(\)\{return (\w+)\|\|(\w+)\|\|/g)]
    .find((m) => code.slice(m.index, m.index + 2400).includes('qoder_auth_wasm'));
  const initMatch = code.match(
    /(\w+)=S\(\(\)=>\{\w+\(\),[^{}]*?JSON\.stringify\(\{uid:"",encrypt_user_info:"",key:""\}\)[^{}]*?\}\)/
  );
  if (!loaderMatch || !initMatch) throw new Error('当前 Qoder 运行时版本不受支持，请更新适配器');
  const wasmLoader = loaderMatch[1];
  const wasmInit = initMatch[1];
  // Report the CLI version the installed runtime itself reports (getVersion fallback).
  const version = code.match(/function \w+\(\)\{return (\w+)&&"UNKNOWN"!==\1\?\1:"([^"]+)"\}/)?.[2] || '1.1.66';
  const hash = createHash('sha256').update(code).digest('hex').slice(0, 20);
  const dir = process.env.AGENTDOCK_QODER_CACHE || join(homedir(), '.agentdock', 'qoder', 'runtime-cache');
  await mkdir(dir, { recursive: true, mode: 0o700 });
  const file = join(dir, `${edition}-${hash}-v2.mjs`);
  code = code.slice(0, at).replaceAll('import.meta.url', JSON.stringify(pathToFileURL(source).href));
  code += `\nexport async function wasmOnly(){${wasmInit}();return await ${wasmLoader}();}\n`;
  try { await readFile(file); } catch {
    const temp = `${file}.${process.pid}.tmp`;
    await writeFile(temp, code, { mode: 0o600 });
    await rename(temp, file);
  }
  const module = await import(pathToFileURL(file).href);
  return { wasm: await module.wasmOnly(), version };
}

async function main(input) {
  const { wasm, version } = await load(input.edition ?? 'intl');
  if (input.op === 'check') return { ok: true, version, capabilities: ['prepare_models', 'decrypt', 'runtime_auth'] };
  if (input.op === 'decrypt') {
    let text = input.text;
    try { text = wasm.decrypt_server_response(text); } catch { /* Plain JSON is also valid. */ }
    return JSON.parse(text);
  }
  const fields = JSON.parse(wasm.generate_runtime_auth_fields(JSON.stringify({
    uid: input.uid, security_oauth_token: input.token, ...(typeof input.dataPolicyAgreed === 'boolean' ? {data_policy_agreed:input.dataPolicyAgreed} : {}),
  })));
  const ctx = new wasm.QoderContext(input.machineId, version, JSON.stringify({
    uid: input.uid, encrypt_user_info: fields.encrypt_user_info, key: fields.key, ...(typeof input.dataPolicyAgreed === 'boolean' ? {data_policy_agreed:input.dataPolicyAgreed} : {}),
  }), JSON.stringify({client_type:'5',business_product:'cli',business_type:'agent',scene:'assistant'}));
  try {
    const result = input.op === 'prepare_chat'
      ? ctx.prepareInferRequest(input.endpoint, JSON.stringify(input.body), input.model, 'system')
      : ctx.prepareRequest(input.endpoint, '/api/v2/model/list?Encode=1', 'GET', 'auth', undefined, undefined);
    try { return { url: result.url, headers: Object.fromEntries(result.headers), ...(input.op === 'prepare_chat' ? {body:result.body} : {}) }; }
    finally { result.free(); }
  } finally { ctx.free(); }
}

const chunks = [];
let size = 0;
for await (const part of process.stdin) {
  size += part.length;
  if (size > 72 * 1024 * 1024) throw new Error('输入超过上限');
  chunks.push(part);
}
const text = Buffer.concat(chunks).toString('utf8');
try {
  const output = await main(JSON.parse(text));
  process.stdout.write(JSON.stringify({ok:true,data:output}));
} catch {
  // Do not echo native exceptions: these can include authentication material.
  process.stdout.write(JSON.stringify({ok:false,error:'Qoder 本地协议运行时不可用，请确认对应版本 Qoder 已安装且与适配器兼容'}));
  process.exitCode = 1;
}
