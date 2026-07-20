fetch(process.env.SOLANA_RPC_URL, {
  method: 'POST', headers: {'Content-Type':'application/json'},
  body: JSON.stringify({jsonrpc:'2.0',id:1,method:'getAsset',params:{id:'JDvQvnUbq8a9BEbXJuNUZiRMXzPsk7EiL1yTzvhXZTnR'}})
}).then(r=>r.json()).then(j=>{
  const a = j.result;
  console.log('name:', a?.content?.metadata?.name);
  console.log('interface:', a?.interface);
  console.log('frozen:', a?.ownership?.frozen);
  console.log('owner:', a?.ownership?.owner);
  console.log('freeze_delegate authority:', a?.plugins?.freeze_delegate?.authority?.address);
  console.log('all plugins:', JSON.stringify(a?.plugins, null, 2));
});