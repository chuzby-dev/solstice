fetch(process.env.SOLANA_RPC_URL, {
  method: 'POST', headers: {'Content-Type':'application/json'},
  body: JSON.stringify({jsonrpc:'2.0',id:1,method:'getTransaction',params:['4aG7EzQZrseoWqZGqF12EmEkuadMUeLz4yL6XuS6akb7jELEsUg95TBcWr3Yhsu8nJa4Wj79guzvRfECbqGuebuC', {maxSupportedTransactionVersion:0}]})
}).then(r=>r.json()).then(j=>{
  console.log(JSON.stringify(j.result.meta.logMessages, null, 2));
});