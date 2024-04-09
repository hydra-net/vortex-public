# Vortex Bot

This trading bot implementation is using the hydranet-api to interface with the Hydranet [LSSD](https://github.com/hydra-net/hydranet-lssd).

There are 3 config files that need to be modified to work with your local LSSD setup.
```
.env.sample -> rename to .env

config.sample.json -> rename to config.json

hydranet-config.sample.json -> rename to hydranet-config.json
```


## Build

```
cargo build --release
```

## Run

### Adjust config files

#### config.json
This file contains trading strategy specific configurations that are independent of the exchange.


```
{
    "settings": {
        "cancel_orders_on_start": true,
        "cancel_orders_on_exit": true
    },
    "strategies": [
        {
            "grid": {
                "exchange": "hydranet", 
                "pair": "HDN_AUSDT", # Pair on the Hydranet DEX
                "grid_profit": 0.02, # = 2%
                "grid_amount": 250,  # Base currency amount. In this case 250HDN
                "min_price": 0.047,  # Lower base currency price in quote currency
                "max_price": 0.053,  # Upper base currency price for the grid in quote currency
                "get_mid_price_from_min_max_avg": true
            }
        } 
    ]
    }
```

#### hyranet-config.json
Here the Hydranet DEX specific configs are stored.

**evm_networks.ethereum.connext_contract** and **evm_networks.arbitrum.connext_contract** needs to be set to your connext channel contract address obtains from the LSSD setup.

**connext_vector_id** is the vector public identifier from the LSSD setup. It starts with vector... .

**lnd_cert_dir and lnd_macaroon_dir** must point to the Cert and Macaroon files generated the the Lightning LND Node.

```
"evm_networks": {
        "ethereum": {
            "connext_contract": "0x1b20389cc4132B5b193e2B810CF912C698B6235E" 
        },
        "arbitrum": {
            "connext_contract": "0x25E278dF64b19D5F4c55B7A1c4d41f20e72eaC0F"
       
   "connext_vector_id": "vector6orthss4amZu1xMpnjEWPTWWNeySboM3qetXX4bma7xL5CAsL4",       

    "lnd_cert_dir": "/vortex/bot-1-lnd-btc/tls.cert",
    "lnd_macaroon_dir": "/vortex/bot-1-lnd-btc/admin.macaroon",
```

### Ensure LSSD is up and running and the channels are funded

launch the built binary
```
./vortex
```

