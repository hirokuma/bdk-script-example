# bdk-script-example

BDKでウォレットを作ってマルチシグにする。

## 準備

RegTestでElectrum Serverを立てる。

* `tcp://localhost:50001`
  * ハードコーディングしているので適当に変更しても良い
* Bitcoin Coreでウォレットを作って、適当にマイニングして自由に扱えるBTCを取得しておく
* BlockstreamのDockerコンテナを使うのが楽だと思うが何でも良い
  * [参考](https://blog.hirokuma.work/bitcoin/01_basics/regtest.html#blockstream-esplora-docker-container)

## 実行

`cargo run` などで立ち上げるとウォレットを作りマルチシグアドレスを出力する。  
その後、マルチシグウォレットの残額変化待ちになる。

出力されたアドレスにBitcoin Coreから `sendtoaddress` で適当に入金する。1BTCでよいだろう。  
そうすると待ち状態が解除される。

解除されると、現在の残額の半分を自分のウォレットに送金するトランザクションを作成する。  
トランザクションのHEX文字列が出力されてマルチシグウォレットの残額変化待ちになる。

Bitcoin Coreから `sendrawtransaction` でそのトランザクションを展開する。  
そうすると待ち状態が解除される。  
そして最初に戻ってマルチシグアドレスを出力する(ウォレットは作成し直さない)。  
これを繰り返すようになっている。

## 補足

ここではマルチシグアドレスを作るのにウォレットを2つ作って拡張公開鍵をminiscriptに与えている。

```
tr(internal_pubkey, multi_a(2, external_pubkey1, external_pubkey2))
```
