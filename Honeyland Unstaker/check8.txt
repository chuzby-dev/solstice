async function inspect(mint, label) {
  const res = await fetch(process.env.SOLANA_RPC_URL, {
    method: 'POST', headers: {'Content-Type':'application/json'},
    body: JSON.stringify({jsonrpc:'2.0',id:1,method:'getAsset',params:{id:mint}})
  });
  const j = await res.json();
  const a = j.result;
  console.log(`\n=== ${label} ===`);
  console.log('compression:', JSON.stringify(a?.compression, null, 2));
}

(async () => {
  await inspect('8tiLduKV23fXh5k6mympFkjtperBwdfZ7h2bXdnACgP', 'WORKING');
  await inspect('JDvQvnUbq8a9BEbXJuNUZiRMXzPsk7EiL1yTzvhXZTnR', 'FAILING');
})();