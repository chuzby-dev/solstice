async function inspect(mint, label) {
  const res = await fetch(process.env.SOLANA_RPC_URL, {
    method: 'POST', headers: {'Content-Type':'application/json'},
    body: JSON.stringify({jsonrpc:'2.0',id:1,method:'getAccountInfo',params:[mint, {encoding:'base64'}]})
  });
  const text = await res.text();
  console.log(`\n=== ${label} (${mint}) ===`);
  console.log('RAW RESPONSE:', text);
}

(async () => {
  await inspect('8tiLduKV23fXh5k6mympFkjtperBwdfZ7h2bXdnACgP', 'WORKING');
  await inspect('JDvQvnUbq8a9BEbXJuNUZiRMXzPsk7EiL1yTzvhXZTnR', 'FAILING');
})();