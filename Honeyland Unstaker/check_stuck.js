async function inspect(sig, label) {
  const res = await fetch(process.env.SOLANA_RPC_URL, {
    method: 'POST', headers: {'Content-Type':'application/json'},
    body: JSON.stringify({jsonrpc:'2.0',id:1,method:'getTransaction',params:[sig, {maxSupportedTransactionVersion:0}]})
  });
  const j = await res.json();
  const tx = j.result;
  console.log(`\n=== ${label} ===`);
  if (!tx) { console.log('NOT FOUND'); return; }
  console.log('blockTime:', new Date(tx.blockTime*1000).toISOString());
  console.log('success:', !tx.meta.err);
  console.log('logs:');
  tx.meta.logMessages.forEach(l => console.log('  ' + l));
}

inspect('3Ayzg5Twxm1EWj2nnMjmgzEETDnNepYRTjAj3yqSdssZFKS6yybhf7C5bX58A3cY9tPmjJWoLWshiy9zNxNTKoWm', 'NEW CHECK');