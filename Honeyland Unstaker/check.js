fetch(process.env.SOLANA_RPC_URL, {
  method: 'POST', headers: {'Content-Type':'application/json'},
  body: JSON.stringify({jsonrpc:'2.0',id:1,method:'getAsset',params:{id:'CN6VbKgfzWx9N3e3E7ctY4GfF1Kf1SaZfG8Utzr7SBij'}})
}).then(r=>r.json()).then(j=>{
  const a = j.result;
  console.log('interface:', a?.interface);
  console.log('frozen:', a?.ownership?.frozen);
  console.log('owner:', a?.ownership?.owner);
  console.log('freeze_delegate authority:', a?.plugins?.freeze_delegate?.authority?.address);
  console.log('collection:', (a?.grouping||[]).find(g=>g.group_key==='collection')?.group_value);
});