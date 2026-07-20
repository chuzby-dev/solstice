function b58encode(buf) {
  const ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
  let digits = [0];
  for (let i = 0; i < buf.length; i++) {
    let carry = buf[i];
    for (let j = 0; j < digits.length; j++) { carry += digits[j] << 8; digits[j] = carry % 58; carry = (carry / 58) | 0; }
    while (carry > 0) { digits.push(carry % 58); carry = (carry / 58) | 0; }
  }
  let result = '';
  for (let i = 0; i < buf.length && buf[i] === 0; i++) result += '1';
  for (let i = digits.length - 1; i >= 0; i--) result += ALPHABET[digits[i]];
  return result;
}

async function inspect(mint, label) {
  const res = await fetch(process.env.SOLANA_RPC_URL, {
    method: 'POST', headers: {'Content-Type':'application/json'},
    body: JSON.stringify({jsonrpc:'2.0',id:1,method:'getAccountInfo',params:[mint, {encoding:'base64'}]})
  });
  const j = await res.json();
  console.log(`\n=== ${label} (${mint}) ===`);
  const rawData = j.result?.value?.data;
  console.log('raw data field type:', Array.isArray(rawData) ? 'array' : typeof rawData);
  const b64 = Array.isArray(rawData) ? rawData[0] : rawData;
  if (!b64) { console.log('NO DATA - value:', JSON.stringify(j.result?.value)); return; }
  const data = Buffer.from(b64, 'base64');
  console.log('total size:', data.length);
  if (data.length >= 66) {
    console.log('key byte (offset 0):', data[0]);
    console.log('owner (offset 1-32):', b58encode(data.subarray(1, 33)));
    console.log('update_authority discriminant (offset 33):', data[33]);
    if (data[33] === 1 || data[33] === 2) {
      console.log('update_authority address (offset 34-65):', b58encode(data.subarray(34, 66)));
    }
  }
}

(async () => {
  await inspect('8tiLduKV23fXh5k6mympFkjtperBwdfZ7h2bXdnACgP', 'WORKING (simulated OK)');
  await inspect('JDvQvnUbq8a9BEbXJuNUZiRMXzPsk7EiL1yTzvhXZTnR', 'FAILING (IncorrectAccount)');
})();