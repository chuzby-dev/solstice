fetch(process.env.SOLANA_RPC_URL, {
  method: 'POST', headers: {'Content-Type':'application/json'},
  body: JSON.stringify({jsonrpc:'2.0',id:1,method:'getAccountInfo',params:['E3fHVpnAWXifNa3odHnWWkk4YdgJJuyKWo5NJNyfLVig', {encoding:'jsonParsed'}]})
}).then(r=>r.json()).then(j=>{
  const v = j.result.value;
  console.log('exists:', !!v);
  console.log('owner:', v?.owner);
  console.log('space:', v?.space);
});