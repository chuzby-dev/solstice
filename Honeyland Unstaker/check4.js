fetch(process.env.SOLANA_RPC_URL, {
  method: 'POST', headers: {'Content-Type':'application/json'},
  body: JSON.stringify({jsonrpc:'2.0',id:1,method:'getAsset',params:{id:'8tiLduKV23fXh5k6mympFkjtperBwdfZ7h2bXdnACgP'}})
}).then(r=>r.json()).then(j=>{
  const a = j.result;
  console.log('name:', a?.content?.metadata?.name);
  console.log('frozen:', a?.ownership?.frozen);
  console.log('collection field:', (a?.grouping||[]).find(g=>g.group_key==='collection')?.group_value);
  console.log('all plugins:', JSON.stringify(a?.plugins, null, 2));
  console.log('updateAuthority:', JSON.stringify(a?.authorities, null, 2));
});