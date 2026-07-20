fetch(process.env.SOLANA_RPC_URL, {
  method: 'POST', headers: {'Content-Type':'application/json'},
  body: JSON.stringify({jsonrpc:'2.0',id:1,method:'getSignaturesForAddress',params:['JDvQvnUbq8a9BEbXJuNUZiRMXzPsk7EiL1yTzvhXZTnR', {limit:10}]})
}).then(r=>r.json()).then(j=>{
  j.result.forEach(s => console.log(s.signature, s.err ? 'FAILED' : 'success', new Date(s.blockTime*1000).toISOString()));
});