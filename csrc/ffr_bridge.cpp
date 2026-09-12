/*
 * waverdi FSDB bridge
 *
 * Thin C ABI layer over the Synopsys FSDB Reader (FFR) C++ SDK. The FFR
 * library only exports C++ symbols, so Rust cannot call it directly; this
 * bridge is compiled with the official headers from
 * $VERDI_HOME/share/FsdbReader (see build.rs) and exposes plain C functions.
 */
#include "ffrAPI.h"

#include <cstdint>
#include <cstring>
#include <new>
#include <string>

namespace {

typedef void (*wav_scope_cb)(void *user, const char *name, const char *module,
                             const char *time_unit, unsigned scope_type);
typedef void (*wav_var_cb)(void *user, const char *name, long long idcode,
                           unsigned lbit, unsigned rbit, unsigned dtidcode,
                           unsigned var_type, unsigned bytes_per_bit);
typedef void (*wav_upscope_cb)(void *user);

struct WavCallbacks {
    wav_scope_cb scope;
    wav_var_cb var;
    wav_upscope_cb upscope;
    void *user;
};

struct WavFsdb {
    ffrObject *obj;
    WavCallbacks cbs;
};

inline WavFsdb *handle(void *h) { return static_cast<WavFsdb *>(h); }
inline ffrVCTrvsHdl vc_handle(void *h) { return static_cast<ffrVCTrvsHdl>(h); }

inline const char *safe(const char *s) { return s ? s : ""; }

bool_T tree_cb(fsdbTreeCBType type, void *client, void *data) {
    auto *fsdb = static_cast<WavFsdb *>(client);
    switch (type) {
    case FSDB_TREE_CBT_SCOPE: {
        auto *scope = static_cast<fsdbScopeRec *>(data);
        fsdb->cbs.scope(fsdb->cbs.user, safe(scope->name), safe(scope->module),
                        safe(scope->time_unit), (unsigned)scope->type);
        break;
    }
    case FSDB_TREE_CBT_VAR: {
        auto *var = static_cast<fsdbVarRec *>(data);
        fsdb->cbs.var(fsdb->cbs.user, safe(var->name),
                      (long long)var->u.idcode, (unsigned)var->lbitnum,
                      (unsigned)var->rbitnum, (unsigned)var->dtidcode,
                      (unsigned)var->type, (unsigned)var->bytes_per_bit);
        break;
    }
    case FSDB_TREE_CBT_UPSCOPE:
        fsdb->cbs.upscope(fsdb->cbs.user);
        break;
    default:
        break;
    }
    return (bool_T)1;
}

inline uint64_t tag_to_u64(const fsdbTag64 &tag) {
    return (static_cast<uint64_t>(tag.H) << 32) | tag.L;
}

inline fsdbTag64 u64_to_tag(uint64_t value) {
    fsdbTag64 tag;
    tag.H = (fsdbTag32)(value >> 32);
    tag.L = (fsdbTag32)(value & 0xffffffffull);
    return tag;
}

// The FFR library prints a banner and warnings to stderr through these
// callbacks; for a full-screen TUI they must be silenced.
int quiet_msg_cb(const char *, ...) { return 0; }

void silence_ffr_messages() {
    ffrObject::ffrRegisterInfoCBFunc(quiet_msg_cb);
    ffrObject::ffrRegisterWarnCBFunc(quiet_msg_cb);
    ffrObject::ffrRegisterErrorCBFunc(quiet_msg_cb);
}

} // namespace

// libnffr resolves some symbols (e.g. sysBusyOn) from libnsys at runtime.
// Record a reference from a function that is definitely kept, so that the
// linker does not drop -lnsys when --as-needed / --gc-sections are active.
extern "C" void sysBusyOn();
void *waverdi_libnsys_anchor = nullptr;

extern "C" {

int wav_fsdb_is_fsdb(const char *path) {
    return ffrObject::ffrIsFSDB((str_T)path) ? 1 : 0;
}

void *wav_fsdb_open(const char *path, wav_scope_cb scope_cb, wav_var_cb var_cb,
                    wav_upscope_cb upscope_cb, void *user) {
    waverdi_libnsys_anchor = (void *)&sysBusyOn;
    silence_ffr_messages();
    ffrObject *obj = ffrObject::ffrOpen3((str_T)path);
    if (!obj) {
        return nullptr;
    }
    WavFsdb *fsdb = new (std::nothrow) WavFsdb{obj, {scope_cb, var_cb, upscope_cb, user}};
    if (!fsdb) {
        obj->ffrClose();
        return nullptr;
    }
    if (obj->ffrSetTreeCBFunc(tree_cb, fsdb) != FSDB_RC_SUCCESS) {
        obj->ffrClose();
        delete fsdb;
        return nullptr;
    }
    return fsdb;
}

int wav_fsdb_file_type(void *h) { return (int)handle(h)->obj->ffrGetFileType(); }

long long wav_fsdb_max_var_idcode(void *h) {
    return (long long)handle(h)->obj->ffrGetMaxVarIdcode();
}

int wav_fsdb_read_tree(void *h) {
    return handle(h)->obj->ffrReadScopeVarTree() == FSDB_RC_SUCCESS ? 0 : -1;
}

int wav_fsdb_add_signal(void *h, long long idcode) {
    return handle(h)->obj->ffrAddToSignalList((fsdbVarIdcode)idcode) ==
                   FSDB_RC_SUCCESS
               ? 0
               : -1;
}

int wav_fsdb_load_signals(void *h) {
    return handle(h)->obj->ffrLoadSignals() == FSDB_RC_SUCCESS ? 0 : -1;
}

int wav_fsdb_time_range(void *h, unsigned long long *min_time,
                        unsigned long long *max_time) {
    fsdbTag64 lo, hi;
    if (handle(h)->obj->ffrGetMinFsdbTag64(&lo) != FSDB_RC_SUCCESS) {
        return -1;
    }
    if (handle(h)->obj->ffrGetMaxFsdbTag64(&hi) != FSDB_RC_SUCCESS) {
        return -1;
    }
    *min_time = tag_to_u64(lo);
    *max_time = tag_to_u64(hi);
    return 0;
}

int wav_fsdb_scale_unit(void *h, char *buf, unsigned long len) {
    str_T unit = handle(h)->obj->ffrGetScaleUnit();
    if (!unit || len == 0) {
        return -1;
    }
    std::strncpy(buf, unit, len - 1);
    buf[len - 1] = '\0';
    return 0;
}

void wav_fsdb_close(void *h) {
    WavFsdb *fsdb = handle(h);
    if (!fsdb) {
        return;
    }
    fsdb->obj->ffrClose();
    delete fsdb;
}

void *wav_fsdb_vc_handle(void *h, long long idcode) {
    return (void *)handle(h)->obj->ffrCreateVCTrvsHdl((fsdbVarIdcode)idcode);
}

int wav_fsdb_has_vc(void *vh) { return vc_handle(vh)->ffrHasIncoreVC() ? 1 : 0; }

int wav_fsdb_min_time(void *vh, unsigned long long *t) {
    fsdbTag64 tag;
    if (vc_handle(vh)->ffrGetMinXTag(&tag) != FSDB_RC_SUCCESS) {
        return -1;
    }
    *t = tag_to_u64(tag);
    return 0;
}

int wav_fsdb_goto_time(void *vh, unsigned long long t) {
    fsdbTag64 tag = u64_to_tag(t);
    return vc_handle(vh)->ffrGotoXTag(&tag) == FSDB_RC_SUCCESS ? 0 : -1;
}

int wav_fsdb_next_vc(void *vh) {
    return vc_handle(vh)->ffrGotoNextVC() == FSDB_RC_SUCCESS ? 0 : -1;
}

int wav_fsdb_cur_time(void *vh, unsigned long long *t) {
    fsdbTag64 tag;
    if (vc_handle(vh)->ffrGetXTag(&tag) != FSDB_RC_SUCCESS) {
        return -1;
    }
    *t = tag_to_u64(tag);
    return 0;
}

int wav_fsdb_value(void *vh, unsigned char *buf, unsigned long len,
                   unsigned long *out_len) {
    ffrVCTrvsHdl hdl = vc_handle(vh);
    byte_T *vc = nullptr;
    if (hdl->ffrGetVC(&vc) != FSDB_RC_SUCCESS || !vc) {
        return -1;
    }
    unsigned bpb = (unsigned)hdl->ffrGetBytesPerBit();
    if (bpb > (unsigned)FSDB_BYTES_PER_BIT_8B) {
        return -1;
    }
    unsigned long size = (bpb == (unsigned)FSDB_BYTES_PER_BIT_1B)
                             ? (unsigned long)hdl->ffrGetBitSize()
                             : (1ul << bpb);
    if (size > len) {
        return -2;
    }
    std::memcpy(buf, vc, size);
    *out_len = size;
    return 0;
}

unsigned wav_fsdb_bit_size(void *vh) { return (unsigned)vc_handle(vh)->ffrGetBitSize(); }

unsigned wav_fsdb_bytes_per_bit(void *vh) {
    return (unsigned)vc_handle(vh)->ffrGetBytesPerBit();
}

unsigned wav_fsdb_var_type(void *vh) {
    return (unsigned)vc_handle(vh)->ffrGetVarType();
}

void wav_fsdb_free_handle(void *vh) {
    if (vh) {
        vc_handle(vh)->ffrFree();
    }
}

} // extern "C"
