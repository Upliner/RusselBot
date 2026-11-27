// RigMoneyModule: a money module for RusselBot's local OpenSim test rig.
//
// Balances live in memory only (every avatar starts with L$1000 when the
// module first meets it, and everything is forgotten when the simulator
// stops). It answers MoneyBalanceRequest and RequestPayPrice, carries out
// MoneyTransferRequest for gifts to avatars (type 5001) and payments to
// objects (type 5008), and tells both parties about each transfer with a
// MoneyBalanceReply whose TransactionInfo is filled in as Second Life fills
// it. A payment to an object goes to the object's owner and raises
// OnObjectPaid, so that the object's scripts get their money() event.
//
// Selected in OpenSim.ini with
//
//   [Economy]
//       economymodule = RigMoneyModule
//
// Console commands:
//
//   rigmoney show [<first> <last>]        balances the module knows
//   rigmoney set <first> <last> <amount>  sets an avatar's balance
//   rigmoney silent on|off                when on, transfers still move the
//                                         money but nobody is told (no
//                                         MoneyBalanceReply), to try
//                                         payments that are never confirmed
//
// Built by test-rig/setup.sh as an addon module (see prebuild.xml).

using System;
using System.Collections.Generic;
using System.Reflection;
using log4net;
using Mono.Addins;
using Nini.Config;
using OpenMetaverse;
using OpenSim.Framework;
using OpenSim.Region.Framework.Interfaces;
using OpenSim.Region.Framework.Scenes;
using OpenSim.Services.Interfaces;

[assembly: Addin("RigMoneyModule", OpenSim.VersionInfo.VersionNumber)]
[assembly: AddinDependency("OpenSim.Region.Framework", OpenSim.VersionInfo.VersionNumber)]

namespace RusselBot.TestRig
{
    [Extension(Path = "/OpenSim/RegionModules", NodeName = "RegionModule", Id = "RigMoneyModule")]
    public class RigMoneyModule : IMoneyModule, ISharedRegionModule
    {
        private static readonly ILog m_log = LogManager.GetLogger(MethodBase.GetCurrentMethod().DeclaringType);

        private const int StartingBalance = 1000;
        private const int Gift = 5001;        // MoneyTransactionType.Gift
        private const int PayObject = 5008;   // MoneyTransactionType.PayObject
        private const int ObjectPays = 5009;  // MoneyTransactionType.ObjectPays (llGiveMoney)

        private bool m_enabled;
        private bool m_silent;
        private bool m_commandsAdded;
        private readonly List<Scene> m_scenes = new List<Scene>();
        private readonly Dictionary<UUID, int> m_balances = new Dictionary<UUID, int>();

        public event ObjectPaid OnObjectPaid;

        public string Name { get { return "RigMoneyModule"; } }
        public Type ReplaceableInterface { get { return null; } }
        public int UploadCharge { get { return 0; } }
        public int GroupCreationCharge { get { return 0; } }

        public void Initialise(IConfigSource config)
        {
            // economymodule may be under [Startup] or (as the example has it) [Economy].
            string module = string.Empty;
            IConfig startup = config.Configs["Startup"];
            if (startup != null)
                module = startup.GetString("economymodule", string.Empty);
            IConfig economy = config.Configs["Economy"];
            if (string.IsNullOrEmpty(module) && economy != null)
                module = economy.GetString("economymodule", economy.GetString("EconomyModule", string.Empty));
            m_enabled = module == Name;
            if (m_enabled)
                m_log.InfoFormat("[RIG MONEY]: enabled; every avatar starts with L${0}", StartingBalance);
        }

        public void PostInitialise() { }
        public void Close() { }

        public void AddRegion(Scene scene)
        {
            if (!m_enabled)
                return;
            scene.RegisterModuleInterface<IMoneyModule>(this);
            lock (m_scenes)
                m_scenes.Add(scene);
            scene.EventManager.OnNewClient += OnNewClient;
            scene.EventManager.OnMoneyTransfer += OnMoneyTransfer;
            scene.EventManager.OnValidateLandBuy += (sender, e) => { lock (e) e.economyValidated = true; };

            if (!m_commandsAdded)
            {
                m_commandsAdded = true;
                MainConsole.Instance.Commands.AddCommand("RigMoney", false, "rigmoney show",
                    "rigmoney show [<first> <last>]", "Show the balances the rig's money module knows", HandleShow);
                MainConsole.Instance.Commands.AddCommand("RigMoney", false, "rigmoney set",
                    "rigmoney set <first> <last> <amount>", "Set an avatar's L$ balance", HandleSet);
                MainConsole.Instance.Commands.AddCommand("RigMoney", false, "rigmoney silent",
                    "rigmoney silent on|off",
                    "When on, transfers move the money but send no MoneyBalanceReply to anyone", HandleSilent);
            }
        }

        public void RegionLoaded(Scene scene) { }

        public void RemoveRegion(Scene scene)
        {
            if (!m_enabled)
                return;
            lock (m_scenes)
                m_scenes.Remove(scene);
            scene.EventManager.OnNewClient -= OnNewClient;
            scene.EventManager.OnMoneyTransfer -= OnMoneyTransfer;
            scene.UnregisterModuleInterface<IMoneyModule>(this);
        }

        private void OnNewClient(IClientAPI client)
        {
            client.OnMoneyBalanceRequest += OnMoneyBalanceRequest;
            client.OnRequestPayPrice += OnRequestPayPrice;
            client.OnEconomyDataRequest += OnEconomyDataRequest;
        }

        // ---- balances

        private int Balance(UUID agent)
        {
            lock (m_balances)
            {
                int balance;
                if (!m_balances.TryGetValue(agent, out balance))
                {
                    balance = StartingBalance;
                    m_balances[agent] = balance;
                }
                return balance;
            }
        }

        /// Moves the money if the payer has enough; returns false otherwise.
        private bool Move(UUID from, UUID to, int amount, out int fromBalance, out int toBalance)
        {
            lock (m_balances)
            {
                fromBalance = Balance(from);
                toBalance = Balance(to);
                if (amount < 0 || fromBalance < amount)
                    return false;
                if (from != to)
                {
                    fromBalance -= amount;
                    toBalance += amount;
                    m_balances[from] = fromBalance;
                    m_balances[to] = toBalance;
                }
                return true;
            }
        }

        // ---- client requests

        private void OnMoneyBalanceRequest(IClientAPI client, UUID agentID, UUID sessionID, UUID transactionID)
        {
            if (client.AgentId != agentID || client.SessionId != sessionID)
                return;
            // A balance with an empty TransactionInfo, as the stock module sends it.
            client.SendMoneyBalance(transactionID, true, Array.Empty<byte>(), Balance(agentID),
                0, UUID.Zero, false, UUID.Zero, false, 0, string.Empty);
        }

        private void OnRequestPayPrice(IClientAPI client, UUID objectID)
        {
            SceneObjectPart part = FindPart(objectID);
            if (part == null)
                return;
            client.SendPayPrice(objectID, part.ParentGroup.RootPart.PayPrice);
        }

        private void OnEconomyDataRequest(IClientAPI client)
        {
            Scene scene = (Scene)client.Scene;
            client.SendEconomyData(1f, scene.RegionInfo.ObjectCapacity, 0, 0, 0,
                0, 0f, 10f, 0, 1f, 0, 0, 0, 0, 0, 0, 2f);
        }

        private void OnMoneyTransfer(object sender, EventManager.MoneyTransferArgs e)
        {
            UUID payer = e.sender;
            int amount = e.amount;
            int type = e.transactiontype;
            string payerName = AgentName(payer);

            if (type != Gift && type != PayObject)
            {
                Fail(payer, type, e.receiver, amount, e.description,
                    string.Format("The rig's money module does not do transactions of type {0}.", type));
                return;
            }
            if (amount <= 0)
            {
                Fail(payer, type, e.receiver, amount, e.description, "The amount must be at least L$1.");
                return;
            }

            UUID payee = e.receiver;
            SceneObjectPart part = null;
            string item = e.description ?? string.Empty;
            if (type == PayObject)
            {
                part = FindPart(e.receiver);
                if (part == null)
                {
                    Fail(payer, type, e.receiver, amount, item, "The object to pay was not found.");
                    return;
                }
                payee = part.OwnerID;
                if (item.Length == 0)
                    item = part.Name;
            }
            else
            {
                if (Account(payee) == null)
                {
                    Fail(payer, type, payee, amount, item, "There is no such avatar.");
                    return;
                }
                if (item.Length == 0)
                    item = "Payment";  // what Second Life sends for a gift without a description
            }
            string payeeName = AgentName(payee);

            int payerBalance, payeeBalance;
            if (!Move(payer, payee, amount, out payerBalance, out payeeBalance))
            {
                Fail(payer, type, payee, amount, item, string.Format(
                    "You cannot pay L${0}: your balance is L${1}.", amount, payerBalance));
                return;
            }

            m_log.InfoFormat("[RIG MONEY]: {0} paid {1} L${2} (type {3}{4}); balances L${5} and L${6}{7}",
                payerName, payeeName, amount, type, part != null ? ", object " + part.Name + " " + part.UUID : "",
                payerBalance, payeeBalance, m_silent ? " (silent)" : "");

            if (!m_silent)
            {
                UUID transaction = UUID.Random();
                string what = part != null ? part.Name + " (" + payeeName + ")" : payeeName;
                Tell(payer, transaction, true, string.Format("You paid {0} L${1}.", what, amount),
                    payerBalance, type, payer, payee, amount, item);
                if (payee != payer)
                    Tell(payee, transaction, true, string.Format("{0} paid you L${1}.", payerName, amount),
                        payeeBalance, type, payer, payee, amount, item);
            }

            if (part != null)
            {
                ObjectPaid handler = OnObjectPaid;
                if (handler != null)
                    handler(part.UUID, payer, amount);
            }
        }

        private void Fail(UUID payer, int type, UUID payee, int amount, string item, string why)
        {
            m_log.InfoFormat("[RIG MONEY]: transfer of L${0} from {1} to {2} (type {3}) refused: {4}{5}",
                amount, AgentName(payer), payee, type, why, m_silent ? " (silent)" : "");
            if (!m_silent)
                Tell(payer, UUID.Random(), false, why, Balance(payer), type, payer, payee, amount, item ?? string.Empty);
        }

        private void Tell(UUID agent, UUID transaction, bool success, string description, int balance,
            int type, UUID source, UUID dest, int amount, string item)
        {
            IClientAPI client = Client(agent);
            if (client != null)
                client.SendMoneyBalance(transaction, success, Utils.StringToBytes(description), balance,
                    type, source, false, dest, false, amount, item);
        }

        // ---- IMoneyModule for the rest of the simulator

        public int GetBalance(UUID agentID) { return Balance(agentID); }
        public bool UploadCovered(UUID agentID, int amount) { return Balance(agentID) >= amount; }
        public bool AmountCovered(UUID agentID, int amount) { return Balance(agentID) >= amount; }
        /// A charge the simulator itself makes, such as a group's fee when
        /// Groups V2 has let the agent join: taken from the balance and
        /// reported with the type it was given and the agent as the payer.
        /// One the agent cannot pay is reported as failed and not taken.
        public void ApplyCharge(UUID agentID, int amount, MoneyTransactionType type, string extraData = "")
        {
            if (amount <= 0)
                return;
            int balance, unused;
            bool paid = Move(agentID, UUID.Zero, amount, out balance, out unused);
            string what = type == MoneyTransactionType.GroupJoin
                ? string.Format("to join the group {0}", extraData)
                : string.Format("for {0} {1}", type, extraData).TrimEnd();
            string description = paid
                ? string.Format("You paid L${0} {1}.", amount, what)
                : "Insufficient funds";
            m_log.InfoFormat("[RIG MONEY]: charge of L${0} to {1} (type {2}) {3}: balance L${4}",
                amount, AgentName(agentID), (int)type, paid ? "taken" : "refused", balance);
            Tell(agentID, UUID.Random(), paid, description, balance, (int)type, agentID, UUID.Zero, amount, extraData ?? string.Empty);
        }
        public void ApplyUploadCharge(UUID agentID, int amount, string text) { }

        public void MoveMoney(UUID fromUser, UUID toUser, int amount, string text)
        {
            MoveMoney(fromUser, toUser, amount, MoneyTransactionType.Gift, text);
        }

        public bool MoveMoney(UUID fromUser, UUID toUser, int amount, MoneyTransactionType type, string text)
        {
            int a, b;
            return Move(fromUser, toUser, amount, out a, out b);
        }

        /// llGiveMoney: the object's owner pays an avatar.
        public bool ObjectGiveMoney(UUID objectID, UUID fromID, UUID toID, int amount, UUID txn, out string reason)
        {
            int fromBalance, toBalance;
            if (!Move(fromID, toID, amount, out fromBalance, out toBalance))
            {
                reason = "LINDENDOLLAR_INSUFFICIENTFUNDS";
                return false;
            }
            reason = string.Empty;
            SceneObjectPart part = FindPart(objectID);
            string item = part != null ? part.Name : string.Empty;
            m_log.InfoFormat("[RIG MONEY]: object {0} of {1} paid {2} L${3}", item, AgentName(fromID), AgentName(toID), amount);
            if (!m_silent)
            {
                Tell(fromID, txn, true, string.Format("{0} paid {1} L${2}.", item, AgentName(toID), amount),
                    fromBalance, ObjectPays, fromID, toID, amount, item);
                if (toID != fromID)
                    Tell(toID, txn, true, string.Format("{0} paid you L${1}.", item, amount),
                        toBalance, ObjectPays, fromID, toID, amount, item);
            }
            return true;
        }

        // ---- console

        private void HandleShow(string module, string[] cmd)
        {
            if (cmd.Length == 4)
            {
                UserAccount account = Account(cmd[2], cmd[3]);
                if (account == null)
                    MainConsole.Instance.Output("No avatar {0} {1}.", cmd[2], cmd[3]);
                else
                    MainConsole.Instance.Output("{0} {1}: L${2}", account.FirstName, account.LastName, Balance(account.PrincipalID));
                return;
            }
            List<KeyValuePair<UUID, int>> all;
            lock (m_balances)
                all = new List<KeyValuePair<UUID, int>>(m_balances);
            if (all.Count == 0)
                MainConsole.Instance.Output("No balances yet; every avatar starts with L${0}.", StartingBalance);
            foreach (KeyValuePair<UUID, int> entry in all)
                MainConsole.Instance.Output("{0} {1}: L${2}", entry.Key, AgentName(entry.Key), entry.Value);
            MainConsole.Instance.Output("Silent: {0}", m_silent ? "on" : "off");
        }

        private void HandleSet(string module, string[] cmd)
        {
            int amount;
            if (cmd.Length != 5 || !int.TryParse(cmd[4], out amount))
            {
                MainConsole.Instance.Output("Usage: rigmoney set <first> <last> <amount>");
                return;
            }
            UserAccount account = Account(cmd[2], cmd[3]);
            if (account == null)
            {
                MainConsole.Instance.Output("No avatar {0} {1}.", cmd[2], cmd[3]);
                return;
            }
            lock (m_balances)
                m_balances[account.PrincipalID] = amount;
            MainConsole.Instance.Output("{0} {1}: L${2}", account.FirstName, account.LastName, amount);
        }

        private void HandleSilent(string module, string[] cmd)
        {
            if (cmd.Length != 3 || (cmd[2] != "on" && cmd[2] != "off"))
            {
                MainConsole.Instance.Output("Usage: rigmoney silent on|off");
                return;
            }
            m_silent = cmd[2] == "on";
            MainConsole.Instance.Output("Silent: {0}", m_silent
                ? "on; transfers move money without a MoneyBalanceReply"
                : "off; transfers are reported to both parties");
        }

        // ---- lookups

        private Scene AnyScene()
        {
            lock (m_scenes)
                return m_scenes.Count > 0 ? m_scenes[0] : null;
        }

        private UserAccount Account(UUID id)
        {
            Scene scene = AnyScene();
            return scene == null ? null : scene.UserAccountService.GetUserAccount(scene.RegionInfo.ScopeID, id);
        }

        private UserAccount Account(string first, string last)
        {
            Scene scene = AnyScene();
            return scene == null ? null : scene.UserAccountService.GetUserAccount(scene.RegionInfo.ScopeID, first, last);
        }

        private string AgentName(UUID id)
        {
            UserAccount account = Account(id);
            return account != null ? account.FirstName + " " + account.LastName : id.ToString();
        }

        private SceneObjectPart FindPart(UUID id)
        {
            lock (m_scenes)
            {
                foreach (Scene scene in m_scenes)
                {
                    SceneObjectPart part = scene.GetSceneObjectPart(id);
                    if (part != null && part.ParentGroup != null && !part.ParentGroup.IsDeleted)
                        return part;
                }
            }
            return null;
        }

        private IClientAPI Client(UUID agent)
        {
            lock (m_scenes)
            {
                foreach (Scene scene in m_scenes)
                {
                    ScenePresence presence = scene.GetScenePresence(agent);
                    if (presence != null && !presence.IsDeleted && !presence.IsChildAgent)
                        return presence.ControllingClient;
                }
            }
            return null;
        }
    }
}
