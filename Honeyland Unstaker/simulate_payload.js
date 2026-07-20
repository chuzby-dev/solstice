const wireTx = "AQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAIFUuan2MwzwCrBpYYYxg1cvoC3SHPp37amqFWMFABHQ0X/5gqMn5Gm3rn/SsUsOtj1SoE8lmptC8ox2a/5Ed3hkiJ5DHtKcJHkTx3vXQf41Dg6LKjST9rYctjAnFeRxAbiAwZGb+UhFzL/7K26csOb57yM5bvF9xJrLEObOkAAAACvVKsQvZelQqCe97OYid0M05SkzOnfps3Jfr4tI1unSG27aCOvWVEJE1KStPzdCAe9hsMFDdSC7OawxE4yNrroAwMACQOghgEAAAAAAAMABQJADQMABAYBAgAEBAQCDAA=";

fetch(process.env.SOLANA_RPC_URL, {
  method: 'POST', headers: {'Content-Type':'application/json'},
  body: JSON.stringify({jsonrpc:'2.0',id:1,method:'simulateTransaction',params:[wireTx, {encoding:'base64', sigVerify:false, replaceRecentBlockhash:true}]})
}).then(r=>r.json()).then(j=>console.log(JSON.stringify(j.result, null, 2)));