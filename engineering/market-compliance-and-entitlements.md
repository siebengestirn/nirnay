# India market access, algo governance and data entitlements

**Review date:** 2026-11-12.  
**This is an engineering control note, not legal advice or a broker eligibility determination.** Have qualified Indian securities-market compliance counsel and the relevant trading member/broker confirm applicability before operating a live strategy or providing trading software/services to others.

## 1. Algo-trading framework

SEBI circular SEBI/HO/MIRSD/MIRSD-PoD/P/CIR/2025/0000013 dated 2025-02-04 established the safer-participation framework for retail investors in algorithmic trading. SEBI's 2025-09-30 timeline extension states that the framework, implementation standards and exchange operational modalities apply to all stock brokers from **2026-04-01**. The current date is after that applicability date. The specific obligations depend on the broker, client, algo, API pathway and whether the system is an in-house tool, vendor product or algo-provider service.

Official sources:

- SEBI extension circular (2025-09-30): <https://www.sebi.gov.in/sebi_data/attachdocs/sep-2025/1759232056254.pdf>
- NSE empanelled algo providers / exchange framework page: <https://www.nseindia.com/static/trade/empanelled-algo-providers-exchange>
- NSE procedure for Decision Support Tools / Algorithms trading: <https://www.nseindia.com/static/trade/platform-services-non-neat-decision-support-tools-algorithm-trading>

NSE's published page currently references circular NSE/INVG/73992 dated 2026-04-30 and says the exchange evaluates algo providers on background, infrastructure and systems. This does not by itself decide whether Nirnay's intended operating model requires a particular registration/empanelment; counsel and the broker must map the exact facts to current circulars and exchange procedures.

## 2. Data rights and account entitlements

API access does not imply permission to store, replay, analyse, redistribute, display or use data for every commercial purpose. For each provider/source, record:

- account/app and entitlement reference without embedding credentials;
- product/segment, instruments, depth/TBT, historical data and delivery mode;
- retention, caching, derived-data, display, redistribution and vendor use rights;
- account holder / beneficial owner and permitted user scope;
- effective period, provider contract/version and approval owner.

Do not mix market-data and order-routing entitlements. The ability to read an order book, retrieve instrument metadata or receive a callback is not a trade permit.

## 3. Runtime conditions

The live pathway is deliberately disabled. No user may turn it on by editing `configs/live.toml`, a Kite private policy, an environment variable or a test permit. A future enablement decision requires current broker approvals, exchange procedures, static network/IP controls where applicable, user authentication, rate limits, strategy registration/identification where required, order tagging, audit evidence, risk controls, incident and mock-session evidence.

This policy must be reviewed whenever SEBI, NSE, BSE, broker API rules, market-data licences, exchange holidays, or the project's operating model changes.
