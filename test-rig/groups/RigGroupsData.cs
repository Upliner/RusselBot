/*
 * The rig's store for OpenSim's Groups V2 (OpenSim.Data.IGroupsData), kept
 * in memory: the rig runs on SQLite, for which OpenSim has no groups store.
 * Everything is gone when the simulator stops; test-rig/group.py makes the
 * groups again. Named under [Groups] as
 *
 *     StorageProvider = "RigGroupsData.dll"
 *
 * The fields of each record are what the MySQL tables of the groups store
 * have (os_groups_Store.migrations); a field the service did not give gets
 * that table's default.
 */

using System;
using System.Collections.Generic;
using System.Linq;
using OpenMetaverse;
using OpenSim.Data;
using OpenSim.Framework;

namespace RusselBot.Rig
{
    public class RigGroupsData : IGroupsData
    {
        private readonly object m_lock = new object();

        private readonly List<GroupData> m_groups = new List<GroupData>();
        private readonly List<MembershipData> m_members = new List<MembershipData>();
        private readonly List<RoleData> m_roles = new List<RoleData>();
        private readonly List<RoleMembershipData> m_roleMembers = new List<RoleMembershipData>();
        private readonly List<PrincipalData> m_principals = new List<PrincipalData>();
        private readonly List<InvitationData> m_invites = new List<InvitationData>();
        private readonly List<NoticeData> m_notices = new List<NoticeData>();

        private static readonly Dictionary<string, string> GroupDefaults = new Dictionary<string, string>
        {
            { "Location", "" }, { "Name", "" }, { "Charter", "" }, { "InsigniaID", UUID.Zero.ToString() },
            { "FounderID", UUID.Zero.ToString() }, { "MembershipFee", "0" }, { "OpenEnrollment", "0" },
            { "ShowInList", "0" }, { "AllowPublish", "0" }, { "MaturePublish", "0" }, { "OwnerRoleID", UUID.Zero.ToString() },
        };
        private static readonly Dictionary<string, string> MemberDefaults = new Dictionary<string, string>
        {
            { "SelectedRoleID", UUID.Zero.ToString() }, { "Contribution", "0" }, { "ListInProfile", "1" },
            { "AcceptNotices", "1" }, { "AccessToken", UUID.Zero.ToString() },
        };
        private static readonly Dictionary<string, string> RoleDefaults = new Dictionary<string, string>
        {
            { "Name", "" }, { "Description", "" }, { "Title", "" }, { "Powers", "0" },
        };
        private static readonly Dictionary<string, string> InviteDefaults = new Dictionary<string, string>
        {
            { "TMStamp", "0" },
        };
        private static readonly Dictionary<string, string> NoticeDefaults = new Dictionary<string, string>
        {
            { "TMStamp", "0" }, { "FromName", "" }, { "Subject", "" }, { "Message", "" }, { "HasAttachment", "0" },
            { "AttachmentType", "0" }, { "AttachmentName", "" }, { "AttachmentItemID", UUID.Zero.ToString() },
            { "AttachmentOwnerID", "" },
        };

        /// <summary>What GroupsServiceBase calls; neither argument is used.</summary>
        public RigGroupsData(string connectionString, string realm)
        {
        }

        /// <summary>A copy of `data` with the table's defaults for what it lacks.</summary>
        private static Dictionary<string, string> Fill(Dictionary<string, string> data, Dictionary<string, string> defaults)
        {
            var filled = new Dictionary<string, string>(defaults);
            if (data != null)
            {
                foreach (var pair in data)
                    filled[pair.Key] = pair.Value ?? "";
            }
            return filled;
        }

        private static GroupData Copy(GroupData g)
        {
            return new GroupData { GroupID = g.GroupID, Data = Fill(g.Data, GroupDefaults) };
        }

        private static MembershipData Copy(MembershipData m)
        {
            return new MembershipData { GroupID = m.GroupID, PrincipalID = m.PrincipalID, Data = Fill(m.Data, MemberDefaults) };
        }

        private static RoleData Copy(RoleData r)
        {
            return new RoleData { GroupID = r.GroupID, RoleID = r.RoleID, Data = Fill(r.Data, RoleDefaults) };
        }

        private static RoleMembershipData Copy(RoleMembershipData r)
        {
            return new RoleMembershipData { GroupID = r.GroupID, RoleID = r.RoleID, PrincipalID = r.PrincipalID };
        }

        private static PrincipalData Copy(PrincipalData p)
        {
            return new PrincipalData { PrincipalID = p.PrincipalID, ActiveGroupID = p.ActiveGroupID };
        }

        private static InvitationData Copy(InvitationData i)
        {
            return new InvitationData
            {
                InviteID = i.InviteID, GroupID = i.GroupID, RoleID = i.RoleID, PrincipalID = i.PrincipalID,
                Data = Fill(i.Data, InviteDefaults)
            };
        }

        private static NoticeData Copy(NoticeData n)
        {
            return new NoticeData { GroupID = n.GroupID, NoticeID = n.NoticeID, Data = Fill(n.Data, NoticeDefaults) };
        }

        private static bool Same(string a, string b)
        {
            return string.Equals(a, b, StringComparison.OrdinalIgnoreCase);
        }

        // ------------------------------------------------------------ groups

        public bool StoreGroup(GroupData data)
        {
            lock (m_lock)
            {
                string name = data.Data != null && data.Data.ContainsKey("Name") ? data.Data["Name"] : "";
                // The table's name is unique.
                if (m_groups.Any(g => g.GroupID != data.GroupID && Same(g.Data["Name"], name)))
                    return false;
                m_groups.RemoveAll(g => g.GroupID == data.GroupID);
                m_groups.Add(Copy(data));
                return true;
            }
        }

        public GroupData RetrieveGroup(UUID groupID)
        {
            lock (m_lock)
            {
                GroupData g = m_groups.FirstOrDefault(x => x.GroupID == groupID);
                return g == null ? null : Copy(g);
            }
        }

        public GroupData RetrieveGroup(string name)
        {
            lock (m_lock)
            {
                GroupData g = m_groups.FirstOrDefault(x => Same(x.Data["Name"], name));
                return g == null ? null : Copy(g);
            }
        }

        public GroupData[] RetrieveGroups(string pattern)
        {
            lock (m_lock)
            {
                return m_groups
                    .Where(g => g.Data["ShowInList"] == "1"
                        && (string.IsNullOrEmpty(pattern) || g.Data["Name"].IndexOf(pattern, StringComparison.OrdinalIgnoreCase) >= 0))
                    .Select(Copy).ToArray();
            }
        }

        public bool DeleteGroup(UUID groupID)
        {
            lock (m_lock)
                return m_groups.RemoveAll(g => g.GroupID == groupID) > 0;
        }

        public int GroupsCount()
        {
            lock (m_lock)
                return m_groups.Count(g => g.Data["Location"] == "");
        }

        // -------------------------------------------------------- membership

        public MembershipData RetrieveMember(UUID groupID, string principalID)
        {
            lock (m_lock)
            {
                MembershipData m = m_members.FirstOrDefault(x => x.GroupID == groupID && Same(x.PrincipalID, principalID));
                return m == null ? null : Copy(m);
            }
        }

        public MembershipData[] RetrieveMembers(UUID groupID)
        {
            lock (m_lock)
                return m_members.Where(m => m.GroupID == groupID).Select(Copy).ToArray();
        }

        public MembershipData[] RetrieveMemberships(string principalID)
        {
            lock (m_lock)
                return m_members.Where(m => Same(m.PrincipalID, principalID)).Select(Copy).ToArray();
        }

        public bool StoreMember(MembershipData data)
        {
            lock (m_lock)
            {
                m_members.RemoveAll(m => m.GroupID == data.GroupID && Same(m.PrincipalID, data.PrincipalID));
                m_members.Add(Copy(data));
                return true;
            }
        }

        public bool DeleteMember(UUID groupID, string principalID)
        {
            lock (m_lock)
                return m_members.RemoveAll(m => m.GroupID == groupID && Same(m.PrincipalID, principalID)) > 0;
        }

        public int MemberCount(UUID groupID)
        {
            lock (m_lock)
                return m_members.Count(m => m.GroupID == groupID);
        }

        // ------------------------------------------------------------- roles

        public bool StoreRole(RoleData data)
        {
            lock (m_lock)
            {
                m_roles.RemoveAll(r => r.GroupID == data.GroupID && r.RoleID == data.RoleID);
                m_roles.Add(Copy(data));
                return true;
            }
        }

        public RoleData RetrieveRole(UUID groupID, UUID roleID)
        {
            lock (m_lock)
            {
                RoleData r = m_roles.FirstOrDefault(x => x.GroupID == groupID && x.RoleID == roleID);
                return r == null ? null : Copy(r);
            }
        }

        public RoleData[] RetrieveRoles(UUID groupID)
        {
            lock (m_lock)
                return m_roles.Where(r => r.GroupID == groupID).Select(Copy).ToArray();
        }

        public bool DeleteRole(UUID groupID, UUID roleID)
        {
            lock (m_lock)
                return m_roles.RemoveAll(r => r.GroupID == groupID && r.RoleID == roleID) > 0;
        }

        public int RoleCount(UUID groupID)
        {
            lock (m_lock)
                return m_roles.Count(r => r.GroupID == groupID);
        }

        // -------------------------------------------------- role membership

        public RoleMembershipData[] RetrieveRolesMembers(UUID groupID)
        {
            lock (m_lock)
                return m_roleMembers.Where(r => r.GroupID == groupID).Select(Copy).ToArray();
        }

        public RoleMembershipData[] RetrieveRoleMembers(UUID groupID, UUID roleID)
        {
            lock (m_lock)
                return m_roleMembers.Where(r => r.GroupID == groupID && r.RoleID == roleID).Select(Copy).ToArray();
        }

        public RoleMembershipData[] RetrieveMemberRoles(UUID groupID, string principalID)
        {
            lock (m_lock)
                return m_roleMembers.Where(r => r.GroupID == groupID && Same(r.PrincipalID, principalID)).Select(Copy).ToArray();
        }

        public RoleMembershipData RetrieveRoleMember(UUID groupID, UUID roleID, string principalID)
        {
            lock (m_lock)
            {
                RoleMembershipData r = m_roleMembers.FirstOrDefault(
                    x => x.GroupID == groupID && x.RoleID == roleID && Same(x.PrincipalID, principalID));
                return r == null ? null : Copy(r);
            }
        }

        public int RoleMemberCount(UUID groupID, UUID roleID)
        {
            lock (m_lock)
                return m_roleMembers.Count(r => r.GroupID == groupID && r.RoleID == roleID);
        }

        public bool StoreRoleMember(RoleMembershipData data)
        {
            lock (m_lock)
            {
                m_roleMembers.RemoveAll(
                    r => r.GroupID == data.GroupID && r.RoleID == data.RoleID && Same(r.PrincipalID, data.PrincipalID));
                m_roleMembers.Add(Copy(data));
                return true;
            }
        }

        public bool DeleteRoleMember(RoleMembershipData data)
        {
            lock (m_lock)
                return m_roleMembers.RemoveAll(
                    r => r.GroupID == data.GroupID && r.RoleID == data.RoleID && Same(r.PrincipalID, data.PrincipalID)) > 0;
        }

        public bool DeleteMemberAllRoles(UUID groupID, string principalID)
        {
            lock (m_lock)
                return m_roleMembers.RemoveAll(r => r.GroupID == groupID && Same(r.PrincipalID, principalID)) > 0;
        }

        // -------------------------------------------------------- principals

        public bool StorePrincipal(PrincipalData data)
        {
            lock (m_lock)
            {
                m_principals.RemoveAll(p => Same(p.PrincipalID, data.PrincipalID));
                m_principals.Add(Copy(data));
                return true;
            }
        }

        public PrincipalData RetrievePrincipal(string principalID)
        {
            lock (m_lock)
            {
                PrincipalData p = m_principals.FirstOrDefault(x => Same(x.PrincipalID, principalID));
                return p == null ? null : Copy(p);
            }
        }

        public bool DeletePrincipal(string principalID)
        {
            lock (m_lock)
                return m_principals.RemoveAll(p => Same(p.PrincipalID, principalID)) > 0;
        }

        // ------------------------------------------------------- invitations

        public bool StoreInvitation(InvitationData data)
        {
            lock (m_lock)
            {
                // One invitation per group and principal, as the table's key.
                m_invites.RemoveAll(i => i.InviteID == data.InviteID
                    || (i.GroupID == data.GroupID && Same(i.PrincipalID, data.PrincipalID)));
                InvitationData stored = Copy(data);
                if (stored.Data["TMStamp"] == "0")
                    stored.Data["TMStamp"] = ((uint)Util.UnixTimeSinceEpoch()).ToString();
                m_invites.Add(stored);
                return true;
            }
        }

        public InvitationData RetrieveInvitation(UUID inviteID)
        {
            lock (m_lock)
            {
                InvitationData i = m_invites.FirstOrDefault(x => x.InviteID == inviteID);
                return i == null ? null : Copy(i);
            }
        }

        public InvitationData RetrieveInvitation(UUID groupID, string principalID)
        {
            lock (m_lock)
            {
                InvitationData i = m_invites.FirstOrDefault(x => x.GroupID == groupID && Same(x.PrincipalID, principalID));
                return i == null ? null : Copy(i);
            }
        }

        public bool DeleteInvite(UUID inviteID)
        {
            lock (m_lock)
                return m_invites.RemoveAll(i => i.InviteID == inviteID) > 0;
        }

        public void DeleteOldInvites()
        {
            // The store lasts as long as the simulator; nothing grows old in it.
        }

        // ----------------------------------------------------------- notices

        public bool StoreNotice(NoticeData data)
        {
            lock (m_lock)
            {
                m_notices.RemoveAll(n => n.NoticeID == data.NoticeID);
                m_notices.Add(Copy(data));
                return true;
            }
        }

        public NoticeData RetrieveNotice(UUID noticeID)
        {
            lock (m_lock)
            {
                NoticeData n = m_notices.FirstOrDefault(x => x.NoticeID == noticeID);
                return n == null ? null : Copy(n);
            }
        }

        public NoticeData[] RetrieveNotices(UUID groupID)
        {
            lock (m_lock)
                return m_notices.Where(n => n.GroupID == groupID).Select(Copy).ToArray();
        }

        public bool DeleteNotice(UUID noticeID)
        {
            lock (m_lock)
                return m_notices.RemoveAll(n => n.NoticeID == noticeID) > 0;
        }

        public void DeleteOldNotices()
        {
        }

        // ------------------------------------------------------ combinations

        public MembershipData RetrievePrincipalGroupMembership(string principalID, UUID groupID)
        {
            // Not implemented by OpenSim's own stores either.
            return null;
        }

        public MembershipData[] RetrievePrincipalGroupMemberships(string principalID)
        {
            return null;
        }
    }
}
