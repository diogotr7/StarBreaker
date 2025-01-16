using System.Collections.Frozen;
using System.Runtime.CompilerServices;
using System.Text;
using StarBreaker.Common;

namespace StarBreaker.DataCore;

public sealed class DataCoreDatabase
{
    private readonly int DataSectionOffset;
    private readonly byte[] DataSection;

    public readonly DataCoreStructDefinition[] StructDefinitions;
    public readonly DataCorePropertyDefinition[] PropertyDefinitions;
    public readonly DataCoreEnumDefinition[] EnumDefinitions;
    public readonly DataCoreDataMapping[] DataMappings;
    public readonly DataCoreRecord[] RecordDefinitions;

    public readonly sbyte[] Int8Values;
    public readonly short[] Int16Values;
    public readonly int[] Int32Values;
    public readonly long[] Int64Values;

    public readonly byte[] UInt8Values;
    public readonly ushort[] UInt16Values;
    public readonly uint[] UInt32Values;
    public readonly ulong[] UInt64Values;

    public readonly bool[] BooleanValues;
    public readonly float[] SingleValues;
    public readonly double[] DoubleValues;
    public readonly CigGuid[] GuidValues;

    public readonly DataCoreStringId[] StringIdValues;
    public readonly DataCoreStringId[] LocaleValues;
    public readonly DataCoreStringId[] EnumValues;

    public readonly DataCorePointer[] StrongValues;
    public readonly DataCorePointer[] WeakValues;
    public readonly DataCoreReference[] ReferenceValues;

    public readonly DataCoreStringId2[] EnumOptions;
    public readonly FrozenSet<CigGuid> MainRecords;

    private readonly FrozenDictionary<int, StructOffsetAndSize> Offsets;
    private readonly DataCorePropertyDefinition[][] Properties;
    private readonly FrozenDictionary<int, string> CachedStrings;
    private readonly FrozenDictionary<int, string> CachedStrings2;
    private readonly FrozenDictionary<CigGuid, DataCoreRecord> RecordMap;

    public DataCoreDatabase(Stream fs)
    {
        using var reader = new BinaryReader(fs);

        _ = reader.ReadUInt32();
        var version = reader.ReadUInt32();
        if (version is < 5 or > 6)
            throw new Exception($"Unsupported file version: {version}");
        _ = reader.ReadUInt32();
        _ = reader.ReadUInt32();

        var structDefinitionCount = reader.ReadInt32();
        var propertyDefinitionCount = reader.ReadInt32();
        var enumDefinitionCount = reader.ReadInt32();
        var dataMappingCount = reader.ReadInt32();
        var recordDefinitionCount = reader.ReadInt32();
        var booleanValueCount = reader.ReadInt32();
        var int8ValueCount = reader.ReadInt32();
        var int16ValueCount = reader.ReadInt32();
        var int32ValueCount = reader.ReadInt32();
        var int64ValueCount = reader.ReadInt32();
        var uint8ValueCount = reader.ReadInt32();
        var uint16ValueCount = reader.ReadInt32();
        var uint32ValueCount = reader.ReadInt32();
        var uint64ValueCount = reader.ReadInt32();
        var singleValueCount = reader.ReadInt32();
        var doubleValueCount = reader.ReadInt32();
        var guidValueCount = reader.ReadInt32();
        var stringIdValueCount = reader.ReadInt32();
        var localeValueCount = reader.ReadInt32();
        var enumValueCount = reader.ReadInt32();
        var strongValueCount = reader.ReadInt32();
        var weakValueCount = reader.ReadInt32();
        var referenceValueCount = reader.ReadInt32();
        var enumOptionCount = reader.ReadInt32();
        var textLength = reader.ReadUInt32();
        var textLength2 = reader.ReadUInt32();

        StructDefinitions = reader.BaseStream.ReadArray<DataCoreStructDefinition>(structDefinitionCount);
        PropertyDefinitions = reader.BaseStream.ReadArray<DataCorePropertyDefinition>(propertyDefinitionCount);
        EnumDefinitions = reader.BaseStream.ReadArray<DataCoreEnumDefinition>(enumDefinitionCount);
        DataMappings = reader.BaseStream.ReadArray<DataCoreDataMapping>(dataMappingCount);
        RecordDefinitions = reader.BaseStream.ReadArray<DataCoreRecord>(recordDefinitionCount);

        Int8Values = reader.BaseStream.ReadArray<sbyte>(int8ValueCount);
        Int16Values = reader.BaseStream.ReadArray<short>(int16ValueCount);
        Int32Values = reader.BaseStream.ReadArray<int>(int32ValueCount);
        Int64Values = reader.BaseStream.ReadArray<long>(int64ValueCount);

        UInt8Values = reader.BaseStream.ReadArray<byte>(uint8ValueCount);
        UInt16Values = reader.BaseStream.ReadArray<ushort>(uint16ValueCount);
        UInt32Values = reader.BaseStream.ReadArray<uint>(uint32ValueCount);
        UInt64Values = reader.BaseStream.ReadArray<ulong>(uint64ValueCount);

        BooleanValues = reader.BaseStream.ReadArray<bool>(booleanValueCount);
        SingleValues = reader.BaseStream.ReadArray<float>(singleValueCount);
        DoubleValues = reader.BaseStream.ReadArray<double>(doubleValueCount);
        GuidValues = reader.BaseStream.ReadArray<CigGuid>(guidValueCount);

        StringIdValues = reader.BaseStream.ReadArray<DataCoreStringId>(stringIdValueCount);
        LocaleValues = reader.BaseStream.ReadArray<DataCoreStringId>(localeValueCount);
        EnumValues = reader.BaseStream.ReadArray<DataCoreStringId>(enumValueCount);

        StrongValues = reader.BaseStream.ReadArray<DataCorePointer>(strongValueCount);
        WeakValues = reader.BaseStream.ReadArray<DataCorePointer>(weakValueCount);
        ReferenceValues = reader.BaseStream.ReadArray<DataCoreReference>(referenceValueCount);
        EnumOptions = reader.BaseStream.ReadArray<DataCoreStringId2>(enumOptionCount);

        CachedStrings = ReadStringTable(reader.ReadBytes((int)textLength).AsSpan());
        if (version >= 6)
            CachedStrings2 = ReadStringTable(reader.ReadBytes((int)textLength2).AsSpan());
        else
            CachedStrings2 = CachedStrings;

        var bytesRead = (int)fs.Position;

        Properties = ReadProperties();
        Offsets = ReadOffsets(bytesRead, DataMappings);
        DataSectionOffset = bytesRead;
        DataSection = reader.ReadBytes((int)(fs.Length - bytesRead));

        RecordMap = RecordDefinitions.ToFrozenDictionary(x => x.Id);

        var mainRecords = new Dictionary<string, DataCoreRecord>();
        foreach (var record in RecordDefinitions)
            mainRecords[record.GetFileName(this)] = record;

        MainRecords = mainRecords.Values.Select(x => x.Id).ToFrozenSet();

#if DEBUG
        DebugGlobal.Database = this;
#endif
    }

    public SpanReader GetReader(int structIndex, int instanceIndex)
    {
        var info = Offsets[structIndex];
        var offset = info.Offset + info.Size * instanceIndex;
        return new SpanReader(DataSection, offset - DataSectionOffset);
    }

    public string GetString(DataCoreStringId id) => CachedStrings[id.Id];
    public string GetString2(DataCoreStringId2 id) => CachedStrings2[id.Id];
    public DataCoreRecord GetRecord(CigGuid guid) => RecordMap[guid];
    public DataCorePropertyDefinition[] GetProperties(int structIndex) => Properties[structIndex];

    private static FrozenDictionary<int, string> ReadStringTable(ReadOnlySpan<byte> span)
    {
        var strings = new Dictionary<int, string>();
        var offset = 0;

        while (offset < span.Length)
        {
            var length = span[offset..].IndexOf((byte)0);
            var useful = span[offset..(offset + length)];
            var str = Encoding.ASCII.GetString(useful);
            strings[offset] = str;
            offset += length + 1;
        }

        return strings.ToFrozenDictionary();
    }

    private FrozenDictionary<int, StructOffsetAndSize> ReadOffsets(int initialOffset, ReadOnlySpan<DataCoreDataMapping> mappings)
    {
        var instances = new Dictionary<int, StructOffsetAndSize>();

        var offset = initialOffset;
        foreach (var mapping in mappings)
        {
            var size = CalculateStructSize(mapping.StructIndex);
            instances[mapping.StructIndex] = new StructOffsetAndSize(offset, size);
            offset += (int)(size * mapping.StructCount);
        }

        return instances.ToFrozenDictionary();
    }

    private DataCorePropertyDefinition[][] ReadProperties()
    {
        var result = new DataCorePropertyDefinition[StructDefinitions.Length][];

        for (var i = 0; i < StructDefinitions.Length; i++)
        {
            result[i] = GetStructProperties(i, this);
        }

        return result;
    }

    private static DataCorePropertyDefinition[] GetStructProperties(int index, DataCoreDatabase db)
    {
        var @this = db.StructDefinitions[index];
        var structs = db.StructDefinitions.AsSpan();
        var properties = db.PropertyDefinitions.AsSpan();

        if (@this is { AttributeCount: 0, ParentTypeIndex: -1 }) return [];

        // Calculate total property count to avoid resizing
        int totalPropertyCount = @this.AttributeCount;
        var baseStruct = @this;
        while (baseStruct.ParentTypeIndex != -1)
        {
            baseStruct = structs[baseStruct.ParentTypeIndex];
            totalPropertyCount += baseStruct.AttributeCount;
        }

        // Pre-allocate array with exact size needed
        var result = new DataCorePropertyDefinition[totalPropertyCount];

        // Reset to start struct for actual property copying
        baseStruct = @this;
        var currentPosition = totalPropertyCount;

        // Copy properties in reverse order to avoid InsertRange
        do
        {
            int count = baseStruct.AttributeCount;
            currentPosition -= count;
            properties.Slice(baseStruct.FirstAttributeIndex, count)
                .CopyTo(result.AsSpan(currentPosition, count));

            if (baseStruct.ParentTypeIndex == -1) break;
            baseStruct = structs[baseStruct.ParentTypeIndex];
        } while (true);

        return result;
    }

    private int CalculateStructSize(int structIndex)
    {
        var size = 0;

        foreach (var attribute in GetProperties(structIndex))
        {
            if (attribute.ConversionType != ConversionType.Attribute)
            {
                //array count + array offset
                size += sizeof(int) * 2;
                continue;
            }

            size += attribute.DataType switch
            {
                DataType.Reference => Unsafe.SizeOf<DataCoreReference>(),
                DataType.WeakPointer => Unsafe.SizeOf<DataCorePointer>(),
                DataType.StrongPointer => Unsafe.SizeOf<DataCorePointer>(),
                DataType.EnumChoice => Unsafe.SizeOf<DataCoreStringId>(),
                DataType.Guid => Unsafe.SizeOf<CigGuid>(),
                DataType.Locale => Unsafe.SizeOf<DataCoreStringId>(),
                DataType.Double => Unsafe.SizeOf<double>(),
                DataType.Single => Unsafe.SizeOf<float>(),
                DataType.String => Unsafe.SizeOf<DataCoreStringId>(),
                DataType.UInt64 => Unsafe.SizeOf<ulong>(),
                DataType.UInt32 => Unsafe.SizeOf<uint>(),
                DataType.UInt16 => Unsafe.SizeOf<ushort>(),
                DataType.Byte => Unsafe.SizeOf<byte>(),
                DataType.Int64 => Unsafe.SizeOf<long>(),
                DataType.Int32 => Unsafe.SizeOf<int>(),
                DataType.Int16 => Unsafe.SizeOf<short>(),
                DataType.SByte => Unsafe.SizeOf<sbyte>(),
                DataType.Boolean => Unsafe.SizeOf<byte>(),
                DataType.Class => CalculateStructSize(attribute.StructIndex),
                _ => throw new InvalidOperationException(nameof(DataType))
            };
        }

        return size;
    }

    public readonly struct StructOffsetAndSize
    {
        public readonly int Offset;
        public readonly int Size;

        public StructOffsetAndSize(int offset, int size)
        {
            Offset = offset;
            Size = size;
        }
    }
}