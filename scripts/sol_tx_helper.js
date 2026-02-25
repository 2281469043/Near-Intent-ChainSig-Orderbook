/**
 * sol_tx_helper.js
 *
 * SOL transaction build and broadcast helper for NEAR MPC Chain Signatures.
 *
 * Subcommands:
 *   build     — Build unsigned SOL transfer, output eddsa_payload for MPC signing
 *   broadcast — Assemble signed tx with MPC EdDSA signature (big_r, s) and broadcast
 *   balance   — Query SOL balance
 *   airdrop   — Request devnet SOL airdrop
 */

const {
  Connection,
  PublicKey,
  Transaction,
  SystemProgram,
  LAMPORTS_PER_SOL,
} = require("@solana/web3.js");
const bs58 = require("bs58").default || require("bs58");

const DEFAULT_RPC = "https://api.devnet.solana.com";

// ============================================================
// Subcommand: balance
// ============================================================
async function cmdBalance(args) {
  const rpcUrl = args[0] || DEFAULT_RPC;
  const address = args[1];
  if (!address) {
    console.error("Usage: node sol_tx_helper.js balance [rpc_url] <address>");
    process.exit(1);
  }
  const connection = new Connection(rpcUrl);
  const balance = await connection.getBalance(new PublicKey(address));
  console.log(
    JSON.stringify({
      lamports: balance.toString(),
      sol: (balance / LAMPORTS_PER_SOL).toFixed(9),
    })
  );
}

// ============================================================
// Subcommand: airdrop
// ============================================================
async function cmdAirdrop(args) {
  const rpcUrl = args[0] || DEFAULT_RPC;
  const address = args[1];
  const solAmount = parseFloat(args[2] || "1");
  if (!address) {
    console.error(
      "Usage: node sol_tx_helper.js airdrop [rpc_url] <address> [amount_sol]"
    );
    process.exit(1);
  }
  const connection = new Connection(rpcUrl, "confirmed");
  const lamports = Math.floor(solAmount * LAMPORTS_PER_SOL);
  console.error(`Requesting airdrop of ${solAmount} SOL to ${address}...`);
  const sig = await connection.requestAirdrop(
    new PublicKey(address),
    lamports
  );
  console.error(`Airdrop signature: ${sig}`);
  console.error("Confirming...");
  const latestBlockhash = await connection.getLatestBlockhash("confirmed");
  await connection.confirmTransaction(
    { signature: sig, ...latestBlockhash },
    "confirmed"
  );
  const newBalance = await connection.getBalance(new PublicKey(address));
  console.log(
    JSON.stringify({
      airdrop_sig: sig,
      lamports_airdropped: lamports.toString(),
      new_balance_lamports: newBalance.toString(),
      new_balance_sol: (newBalance / LAMPORTS_PER_SOL).toFixed(9),
    })
  );
}

// ============================================================
// Subcommand: build
// ============================================================
async function cmdBuild(args) {
  const rpcUrl = args[0] || DEFAULT_RPC;
  const from = args[1];
  const to = args[2];
  const lamports = args[3];

  if (!from || !to || !lamports) {
    console.error(
      "Usage: node sol_tx_helper.js build [rpc_url] <from> <to> <lamports>"
    );
    process.exit(1);
  }

  const connection = new Connection(rpcUrl);
  const fromPubkey = new PublicKey(from);
  const toPubkey = new PublicKey(to);

  const transaction = new Transaction().add(
    SystemProgram.transfer({
      fromPubkey,
      toPubkey,
      lamports: BigInt(lamports),
    })
  );

  const { blockhash, lastValidBlockHeight } =
    await connection.getLatestBlockhash("finalized");
  transaction.recentBlockhash = blockhash;
  transaction.lastValidBlockHeight = lastValidBlockHeight;
  transaction.feePayer = fromPubkey;

  const messageBytes = transaction.serializeMessage();

  console.log(
    JSON.stringify({
      eddsa_payload: Array.from(messageBytes),
      payload: new Array(32).fill(0),
      message_bytes_hex: Buffer.from(messageBytes).toString("hex"),
      message_length: messageBytes.length,
      blockhash,
      last_valid_block_height: lastValidBlockHeight,
      from,
      to,
      lamports,
      transaction_base64: transaction
        .serialize({ requireAllSignatures: false, verifySignatures: false })
        .toString("base64"),
    })
  );
}

// ============================================================
// Subcommand: broadcast
// ============================================================
async function cmdBroadcast(args) {
  const rpcUrl = args[0] || DEFAULT_RPC;
  const txBase64 = args[1];
  const senderAddress = args[2];
  const bigR = args[3];
  const s = args[4];

  if (!txBase64 || !senderAddress || !bigR || !s) {
    console.error(
      "Usage: node sol_tx_helper.js broadcast [rpc_url] <tx_base64> <sender_address> <big_r_hex> <s_hex>"
    );
    process.exit(1);
  }

  const transaction = Transaction.from(Buffer.from(txBase64, "base64"));

  let rHex = bigR.startsWith("0x") ? bigR.slice(2) : bigR;
  let sHex = s.startsWith("0x") ? s.slice(2) : s;
  if (rHex.length === 66) rHex = rHex.slice(2);
  if (sHex.length === 66) sHex = sHex.slice(2);

  if (rHex.length !== 64 || sHex.length !== 64) {
    console.error(
      `Invalid EdDSA signature: R=${rHex.length}chars, S=${sHex.length}chars (expected 64 each)`
    );
    process.exit(1);
  }

  const rBytes = Buffer.from(rHex, "hex");
  const sBytes = Buffer.from(sHex, "hex");
  const signature = Buffer.concat([rBytes, sBytes]);

  const senderPubkey = new PublicKey(senderAddress);
  const sigIndex = transaction.signatures.findIndex((pair) =>
    pair.publicKey.equals(senderPubkey)
  );
  if (sigIndex < 0) {
    console.error(`Sender ${senderAddress} not in transaction signers`);
    process.exit(1);
  }
  transaction.signatures[sigIndex].signature = signature;

  const signedSerialized = transaction.serialize({
    requireAllSignatures: true,
    verifySignatures: false,
  });

  const signatureBase58 = bs58.encode(signature);
  console.error(`Signed tx size: ${signedSerialized.length} bytes`);
  console.error(`Signature: ${signatureBase58.slice(0, 30)}...`);
  console.error("Broadcasting...");

  const connection = new Connection(rpcUrl, "confirmed");
  try {
    const txHash = await connection.sendRawTransaction(signedSerialized, {
      skipPreflight: false,
      preflightCommitment: "confirmed",
    });
    console.error(`Broadcast success! Signature: ${txHash}`);
    console.error("Confirming...");

    const latestBlockhash = await connection.getLatestBlockhash("confirmed");
    const confirmation = await connection.confirmTransaction(
      {
        signature: txHash,
        blockhash: latestBlockhash.blockhash,
        lastValidBlockHeight: latestBlockhash.lastValidBlockHeight,
      },
      "confirmed"
    );

    const status = confirmation.value?.err ? "failed" : "success";
    console.log(
      JSON.stringify({
        tx_hash: txHash,
        status,
        error: confirmation.value?.err || null,
      })
    );
  } catch (err) {
    console.error(`Broadcast failed: ${err.message}`);
    console.log(
      JSON.stringify({
        error: err.message,
        signed_base64: signedSerialized.toString("base64"),
        signature: signatureBase58,
      })
    );
    process.exit(1);
  }
}

// ============================================================
// Main
// ============================================================
async function main() {
  const cmd = process.argv[2];
  const args = process.argv.slice(3);

  switch (cmd) {
    case "balance":
      await cmdBalance(args);
      break;
    case "airdrop":
      await cmdAirdrop(args);
      break;
    case "build":
      await cmdBuild(args);
      break;
    case "broadcast":
      await cmdBroadcast(args);
      break;
    default:
      console.error(
        "Usage: node sol_tx_helper.js <balance|airdrop|build|broadcast> ..."
      );
      console.error("  balance   [rpc_url] <address>");
      console.error("  airdrop   [rpc_url] <address> [amount_sol]");
      console.error("  build     [rpc_url] <from> <to> <lamports>");
      console.error(
        "  broadcast [rpc_url] <tx_base64> <sender> <big_r_hex> <s_hex>"
      );
      process.exit(1);
  }
}

main().catch((err) => {
  console.error("Error:", err.message || err);
  process.exit(1);
});
